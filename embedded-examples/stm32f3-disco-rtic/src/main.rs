#![no_std]
#![no_main]
use satrs::pus::verification::{FailParams, VerificationReportCreator};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use satrs::spacepackets::ecss::EcssEnumU16;
use satrs::spacepackets::CcsdsPacket;
use satrs::spacepackets::{ByteConversionError, SpHeader};
// global logger + panicking-behavior + memory layout
use satrs_stm32f3_disco_rtic as _;

use rtic::app;

use heapless::Vec;
#[allow(unused_imports)]
use rtic_monotonics::fugit::{MillisDurationU32, TimerInstantU32};
use rtic_monotonics::systick::prelude::*;
use satrs::spacepackets::{ecss::PusPacket, ecss::WritablePusPacket};
use spacepackets::seq_count::SequenceCountProvider;

const UART_BAUD: u32 = 115200;
const DEFAULT_BLINK_FREQ_MS: u32 = 1000;
const TX_HANDLER_FREQ_MS: u32 = 20;
const MIN_DELAY_BETWEEN_TX_PACKETS_MS: u32 = 5;
const MAX_TC_LEN: usize = 128;
const MAX_TM_LEN: usize = 128;

pub const PUS_APID: u16 = 0x02;

// This is the predictable maximum overhead of the COBS encoding scheme.
// It is simply the maximum packet lenght dividied by 254 rounded up.
const COBS_TC_OVERHEAD: usize = cobs::max_encoding_overhead(MAX_TC_LEN);
const COBS_TM_OVERHEAD: usize = cobs::max_encoding_overhead(MAX_TM_LEN);

const TC_BUF_LEN: usize = MAX_TC_LEN + COBS_TC_OVERHEAD;
const TM_BUF_LEN: usize = MAX_TC_LEN + COBS_TM_OVERHEAD;

const TC_DMA_BUF_LEN: usize = 512;

type TmPacket = heapless::Vec<u8, MAX_TM_LEN>;
type TcPacket = heapless::Vec<u8, MAX_TC_LEN>;

static TM_REQUESTS: static_cell::ConstStaticCell<heapless::spsc::Queue<TmPacket, 8>> =
    static_cell::ConstStaticCell::new(heapless::spsc::Queue::new());

use core::sync::atomic::{AtomicU16, Ordering};

pub struct SeqCountProviderAtomicRef {
    atomic: AtomicU16,
    ordering: Ordering,
}

impl SeqCountProviderAtomicRef {
    pub const fn new(ordering: Ordering) -> Self {
        Self {
            atomic: AtomicU16::new(0),
            ordering,
        }
    }
}

impl SequenceCountProvider for SeqCountProviderAtomicRef {
    type Raw = u16;
    const MAX_BIT_WIDTH: usize = 16;

    fn get(&self) -> u16 {
        self.atomic.load(self.ordering)
    }

    fn increment(&self) {
        self.atomic.fetch_add(1, self.ordering);
    }

    fn get_and_increment(&self) -> u16 {
        self.atomic.fetch_add(1, self.ordering)
    }
}

static SEQ_COUNT_PROVIDER: SeqCountProviderAtomicRef =
    SeqCountProviderAtomicRef::new(Ordering::Relaxed);

pub struct TxIdle {
    //tx: TxType,
    //dma_channel: dma1::C7,
}

#[derive(Debug, defmt::Format)]
pub enum TmSendError {
    ByteConversion(ByteConversionError),
    Queue,
}

impl From<ByteConversionError> for TmSendError {
    fn from(value: ByteConversionError) -> Self {
        Self::ByteConversion(value)
    }
}

pub struct RequestWithToken {
    request_id: satrs::pus::verification::RequestId,
    request: Request,
}

#[derive(Debug, defmt::Format)]
pub enum Request {
    Ping,
    ChangeBlinkFrequency(u32),
}

#[derive(Debug, defmt::Format)]
pub enum RequestError {
    InvalidApid = 1,
    InvalidService = 2,
    InvalidSubservice = 3,
    NotEnoughAppData = 4,
}

#[app(device = embassy_stm32)]
mod app {
    use super::*;
    use satrs::pus::verification::VerificationReportCreator;
    use satrs::spacepackets::{ecss::tc::PusTcReader, time::cds::P_FIELD_BASE};
    use satrs_stm32f3_disco_rtic::LedPinSet;

    systick_monotonic!(Mono, 1000);

    embassy_stm32::bind_interrupts!(struct Irqs {
        USART2 => embassy_stm32::usart::InterruptHandler<embassy_stm32::peripherals::USART2>;
    });

    #[shared]
    struct Shared {
        blink_freq: MillisDurationU32,
    }

    #[local]
    struct Local {
        verif_reporter: VerificationReportCreator,
        leds: satrs_stm32f3_disco_rtic::Leds,
        current_dir: satrs_stm32f3_disco_rtic::Direction,
        tm_prod: heapless::spsc::Producer<'static, TmPacket>,
        tm_cons: heapless::spsc::Consumer<'static, TmPacket>,
        tx: embassy_stm32::usart::UartTx<'static, embassy_stm32::mode::Async>,
        rx: embassy_stm32::usart::RingBufferedUartRx<'static>,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        static DMA_BUF: static_cell::ConstStaticCell<[u8; TC_DMA_BUF_LEN]> = static_cell::ConstStaticCell::new([0; TC_DMA_BUF_LEN]);

        let p = embassy_stm32::init(Default::default());

        // Initialize the systick interrupt & obtain the token to prove that we did
        Mono::start(cx.core.SYST, 8_000_000);

        defmt::info!("Starting sat-rs demo application for the STM32F3-Discovery with RTICv2");
        let led_pin_set = LedPinSet {
            pin_n: p.PE8,
            pin_ne: p.PE9,
            pin_e: p.PE10,
            pin_se: p.PE11,
            pin_s: p.PE12,
            pin_sw: p.PE13,
            pin_w: p.PE14,
            pin_nw: p.PE15,
        };
        let leds = satrs_stm32f3_disco_rtic::Leds::new(led_pin_set);

        let config = embassy_stm32::usart::Config::default();
        let uart = embassy_stm32::usart::Uart::new(
            p.USART2, p.PA3, p.PA2, Irqs, p.DMA1_CH7, p.DMA1_CH6, config,
        )
        .unwrap();

        let (tx, rx) = uart.split();
        let tm_queue = TM_REQUESTS.take();
        let (tm_prod, tm_cons) = tm_queue.split();
        //let mut gpioa = cx.device.GPIOA.split(&mut rcc.ahb);
        // USART2 pins
        /*
        let mut pins = (
            // TX pin: PA2
            gpioa
                .pa2
                .into_af_push_pull(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrl),
            // RX pin: PA3
            gpioa
                .pa3
                .into_af_push_pull(&mut gpioa.moder, &mut gpioa.otyper, &mut gpioa.afrl),
        );
        */
        //pins.1.internal_pull_up(&mut gpioa.pupdr, true);
        /*
                let mut usart2 = Serial::new(
                    cx.device.USART2,
                    pins,
                    UART_BAUD.Bd(),
                    clocks,
                    &mut rcc.apb1,
                );
                usart2.configure_rx_interrupt(RxEvent::Idle, Switch::On);
                // This interrupt is enabled to re-schedule new transfers in the interrupt handler immediately.
                usart2.configure_tx_interrupt(TxEvent::TransmissionComplete, Switch::On);
        */

        //let dma1 = cx.device.DMA1.split(&mut rcc.ahb);
        //let (mut tx_serial, mut rx_serial) = usart2.split();

        // This interrupt is immediately triggered, clear it. It will only be reset
        // by the hardware when data is received on RX (RXNE event)
        //rx_serial.clear_event(RxEvent::Idle);
        // For some reason, this is also immediately triggered..
        //tx_serial.clear_event(TxEvent::TransmissionComplete);
        //let rx_transfer = rx_serial.read_exact(unsafe { DMA_RX_BUF.as_mut_slice() }, dma1.ch6);
        defmt::info!("Spawning tasks");
        blinky::spawn().unwrap();
        serial_tx_handler::spawn().unwrap();

        let verif_reporter = VerificationReportCreator::new(PUS_APID).unwrap();

        (
            Shared {
                blink_freq: MillisDurationU32::from_ticks(DEFAULT_BLINK_FREQ_MS),
            },
            Local {
                verif_reporter,
                leds,
                tm_prod,
                tm_cons,
                tx,
                rx: rx.into_ring_buffered(DMA_BUF.take()),
                current_dir: satrs_stm32f3_disco_rtic::Direction::North,
            },
        )
    }

    #[task(local = [leds, current_dir], shared=[blink_freq])]
    async fn blinky(mut cx: blinky::Context) {
        loop {
            cx.local.leds.blink_next(cx.local.current_dir);
            let current_blink_freq = cx.shared.blink_freq.lock(|current| *current);
            Mono::delay(current_blink_freq).await;
        }
    }

    #[task(
        local = [
            tm_cons,
            tx,
            dma_buf: [u8; TM_BUF_LEN] = [0; TM_BUF_LEN]
        ],
        shared = [],
    )]
    async fn serial_tx_handler(cx: serial_tx_handler::Context) {
        loop {
            while let Some(vec) = cx.local.tm_cons.dequeue() {
                cx.local.dma_buf[0] = 0;
                let encoded_len = cobs::encode(&vec[0..vec.len()], &mut cx.local.dma_buf[1..]);
                // Should never panic, we accounted for the overhead.
                // Write into transfer buffer directly, no need for intermediate
                // encoding buffer.
                // 0 end marker
                cx.local.dma_buf[encoded_len + 1] = 0;
                cx.local.tx.write(&vec[0..encoded_len + 2]).await.unwrap();
                continue;
            }
            Mono::delay(TX_HANDLER_FREQ_MS.millis()).await;
        }
    }

    #[task(
        local = [
            verif_reporter,
            tm_prod,
            rx,
            read_buf: [u8; 128] = [0; 128],
            decode_buf: [u8; MAX_TC_LEN] = [0; MAX_TC_LEN],
            src_data_buf: [u8; MAX_TM_LEN] = [0; MAX_TM_LEN],
            timestamp: [u8; 7] = [0; 7],
        ],
        shared = [blink_freq]
    )]
    async fn serial_rx_handler(
        mut cx: serial_rx_handler::Context,
        received_packet: Vec<u8, MAX_TC_LEN>,
    ) {
        let decoder = cobs::CobsDecoder::new(cx.local.decode_buf);
        loop {
            let read_bytes = cx.local.rx.read(cx.local.read_buf).await;
            match decoder.push(&cx.local.read_buf[0..read_bytes]) {
                Ok(None) => (),
                Ok(Some(report)) => {

                }
                Err(_) => {},
            }

        }
        cx.local.timestamp[0] = P_FIELD_BASE;
        defmt::info!("Received packet with {} bytes", received_packet.len());
        let decode_buf = cx.local.decode_buf;
        let packet = received_packet.as_slice();
        let mut start_idx = None;
        for (idx, byte) in packet.iter().enumerate() {
            if *byte != 0 {
                start_idx = Some(idx);
                break;
            }
        }
        if start_idx.is_none() {
            defmt::warn!("decoding error, can only process cobs encoded frames, data is all 0");
            return;
        }
        let start_idx = start_idx.unwrap();
        decoder.push(data)
        match cobs::decode(&received_packet.as_slice()[start_idx..], decode_buf) {
            Ok(decode_report) => {
                defmt::info!("Decoded packet length: {}", decode_report.frame_size());
                let pus_tc = PusTcReader::new(decode_buf);
                match pus_tc {
                    Ok(tc) => {
                        match convert_pus_tc_to_request(
                            &tc,
                            cx.local.verif_reporter,
                            cx.local.tm_prod,
                            cx.local.src_data_buf,
                            cx.local.timestamp,
                        ) {
                            Ok(request_with_token) => {
                                handle_start_verification(
                                    request_with_token.request_id,
                                    cx.local.tm_prod,
                                    cx.local.verif_reporter,
                                    cx.local.src_data_buf,
                                    cx.local.timestamp,
                                );

                                match request_with_token.request {
                                    Request::Ping => {
                                        handle_ping_request(cx.local.tm_prod, cx.local.timestamp);
                                    }
                                    Request::ChangeBlinkFrequency(new_freq_ms) => {
                                        defmt::info!("Received blink frequency change request with new frequncy {}", new_freq_ms);
                                        cx.shared.blink_freq.lock(|blink_freq| {
                                            *blink_freq =
                                                MillisDurationU32::from_ticks(new_freq_ms);
                                        });
                                    }
                                }
                                handle_completion_verification(
                                    request_with_token.request_id,
                                    cx.local.tm_prod,
                                    cx.local.verif_reporter,
                                    cx.local.src_data_buf,
                                    cx.local.timestamp,
                                );
                            }
                            Err(e) => {
                                // TODO: Error handling: Send verification failure based on request error.
                                defmt::warn!("request error {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        defmt::warn!("Error unpacking PUS TC: {}", e);
                    }
                }
            }
            Err(_) => {
                defmt::warn!("decoding error, can only process cobs encoded frames")
            }
        }
    }

    fn handle_ping_request(
        tm_prod: &mut heapless::spsc::Producer<'static, TmPacket>,
        timestamp: &[u8],
    ) {
        defmt::info!("Received PUS ping telecommand, sending ping reply TM[17,2]");
        let sp_header =
            SpHeader::new_for_unseg_tc(PUS_APID, SEQ_COUNT_PROVIDER.get_and_increment(), 0);
        let sec_header = PusTmSecondaryHeader::new_simple(17, 2, timestamp);
        let ping_reply = PusTmCreator::new(sp_header, sec_header, &[], true);
        let mut tm_packet = TmPacket::new();
        tm_packet
            .resize(ping_reply.len_written(), 0)
            .expect("vec resize failed");
        ping_reply.write_to_bytes(&mut tm_packet).unwrap();
        if tm_prod.enqueue(tm_packet).is_err() {
            defmt::warn!("TC queue full");
        }
    }

    fn handle_start_verification(
        request_id: satrs::pus::verification::RequestId,
        tm_prod: &mut heapless::spsc::Producer<TmPacket>,
        verif_reporter: &mut VerificationReportCreator,
        src_data_buf: &mut [u8],
        timestamp: &[u8],
    ) {
        let tm_creator = verif_reporter
            .start_success(
                src_data_buf,
                &request_id,
                SEQ_COUNT_PROVIDER.get(),
                0,
                timestamp,
            )
            .unwrap();
        let result = send_tm(tm_creator, tm_prod);
        if let Err(e) = result {
            handle_tm_send_error(e);
        }
    }

    fn handle_completion_verification(
        request_id: satrs::pus::verification::RequestId,
        tm_prod: &mut heapless::spsc::Producer<TmPacket>,
        verif_reporter: &mut VerificationReportCreator,
        src_data_buf: &mut [u8],
        timestamp: &[u8],
    ) {
        let result = send_tm(
            verif_reporter
                .completion_success(
                    src_data_buf,
                    &request_id,
                    SEQ_COUNT_PROVIDER.get(),
                    0,
                    timestamp,
                )
                .unwrap(),
            tm_prod,
        );
        if let Err(e) = result {
            handle_tm_send_error(e);
        }
    }

    /*
    #[task(binds = DMA1_CH6, shared = [])]
    fn rx_dma_isr(cx: rx_dma_isr::Context) {
        /*
        let mut tc_packet = TcPacket::new();
        cx.shared.rx_transfer.lock(|rx_transfer| {
            let rx_ref = rx_transfer.as_ref().unwrap();
            if rx_ref.is_complete() {
                let uart_rx_owned = rx_transfer.take().unwrap();
                let (buf, c, rx) = uart_rx_owned.stop();
                // The received data is transferred to another task now to avoid any processing overhead
                // during the interrupt. There are multiple ways to do this, we use a stack allocaed vector here
                // to do this.
                tc_packet.resize(buf.len(), 0).expect("vec resize failed");
                tc_packet.copy_from_slice(buf);

                // Start the next transfer as soon as possible.
                *rx_transfer = Some(rx.read_exact(buf, c));

                // Send the vector to a regular task.
                serial_rx_handler::spawn(tc_packet).expect("spawning rx handler task failed");
                // If this happens, there is a high chance that the maximum packet length was
                // exceeded. Circular mode is not used here, so data might be missed.
                defmt::warn!(
                    "rx transfer with maximum length {}, might miss data",
                    TC_BUF_LEN
                );
            }
        });
        */
    }
    */

    /*
    #[task(binds = USART2_EXTI26, shared = [])]
    fn serial_isr(mut cx: serial_isr::Context) {
        /*
                cx.shared
                    .tx_shared
                    .lock(|tx_shared| match &mut tx_shared.state {
                        UartTxState::Idle(_) => (),
                        UartTxState::Transmitting(transfer) => {
                            let transfer_ref = transfer.as_ref().unwrap();
                            if transfer_ref.is_complete() {
                                let transfer = transfer.take().unwrap();
                                let (_, dma_channel, mut tx) = transfer.stop();
                                tx.clear_event(TxEvent::TransmissionComplete);
                                tx_shared.state = UartTxState::Idle(Some(TxIdle { tx, dma_channel }));
                                // We cache the last completed time to ensure that there is a minimum delay between consecutive
                                // transferred packets.
                                tx_shared.last_completed = Some(Mono::now());
                            }
                        }
                    });
                let mut tc_packet = TcPacket::new();
                cx.shared.rx_transfer.lock(|rx_transfer| {
                    let rx_transfer_ref = rx_transfer.as_ref().unwrap();
                    // Received a partial packet.
                    if rx_transfer_ref.is_event_triggered(RxEvent::Idle) {
                        let rx_transfer_owned = rx_transfer.take().unwrap();
                        let (buf, ch, mut rx, rx_len) = rx_transfer_owned.stop_and_return_received_bytes();
                        // The received data is transferred to another task now to avoid any processing overhead
                        // during the interrupt. There are multiple ways to do this, we use a stack
                        // allocated vector to do this.
                        tc_packet
                            .resize(rx_len as usize, 0)
                            .expect("vec resize failed");
                        tc_packet[0..rx_len as usize].copy_from_slice(&buf[0..rx_len as usize]);
                        rx.clear_event(RxEvent::Idle);
                        serial_rx_handler::spawn(tc_packet).expect("spawning rx handler failed");
                        *rx_transfer = Some(rx.read_exact(buf, ch));
                    }
                });
        */
    }
    */
}

fn send_tm(
    tm_creator: PusTmCreator,
    tm_prod: &mut heapless::spsc::Producer<TmPacket>,
) -> Result<(), TmSendError> {
    if tm_creator.len_written() > MAX_TM_LEN {
        return Err(ByteConversionError::ToSliceTooSmall {
            expected: tm_creator.len_written(),
            found: MAX_TM_LEN,
        }
        .into());
    }
    let mut tm_vec = TmPacket::new();
    tm_vec
        .resize(tm_creator.len_written(), 0)
        .expect("vec resize failed");
    tm_creator.write_to_bytes(tm_vec.as_mut_slice())?;
    defmt::info!(
        "Sending TM[{},{}] with size {}",
        tm_creator.service(),
        tm_creator.subservice(),
        tm_creator.len_written()
    );
    tm_prod.enqueue(tm_vec).map_err(|_| TmSendError::Queue)?;
    Ok(())
}

fn handle_tm_send_error(error: TmSendError) {
    defmt::warn!("sending tm failed with error {}", error);
}

pub fn convert_pus_tc_to_request(
    tc: &PusTcReader,
    verif_reporter: &mut VerificationReportCreator,
    tm_prod: &mut heapless::spsc::Producer<'static, TmPacket>,
    src_data_buf: &mut [u8],
    timestamp: &[u8],
) -> Result<RequestWithToken, RequestError> {
    defmt::info!(
        "Found PUS TC [{},{}] with length {}",
        tc.service(),
        tc.subservice(),
        tc.len_packed()
    );

    let request_id = verif_reporter.read_request_id(tc);
    if tc.apid() != PUS_APID {
        defmt::warn!("Received tc with unknown APID {}", tc.apid());
        let result = send_tm(
            verif_reporter
                .acceptance_failure(
                    src_data_buf,
                    &request_id,
                    SEQ_COUNT_PROVIDER.get_and_increment(),
                    0,
                    FailParams::new(timestamp, &EcssEnumU16::new(0), &[]),
                )
                .unwrap(),
            tm_prod,
        );
        if let Err(e) = result {
            handle_tm_send_error(e);
        }
        return Err(RequestError::InvalidApid);
    }
    let tm_creator = verif_reporter
        .acceptance_success(
            src_data_buf,
            &request_id,
            SEQ_COUNT_PROVIDER.get_and_increment(),
            0,
            timestamp,
        )
        .unwrap();

    if let Err(e) = send_tm(tm_creator, tm_prod) {
        handle_tm_send_error(e);
    }

    if tc.service() == 17 && tc.subservice() == 1 {
        if tc.subservice() == 1 {
            Ok(RequestWithToken {
                request_id,
                request: Request::Ping,
            })
        } else {
            Err(RequestError::InvalidSubservice)
        }
    } else if tc.service() == 8 {
        if tc.subservice() == 1 {
            if tc.user_data().len() < 4 {
                return Err(RequestError::NotEnoughAppData);
            }
            let new_freq_ms = u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap());
            Ok(RequestWithToken {
                request_id,
                request: Request::ChangeBlinkFrequency(new_freq_ms),
            })
        } else {
            Err(RequestError::InvalidSubservice)
        }
    } else {
        Err(RequestError::InvalidService)
    }
}
