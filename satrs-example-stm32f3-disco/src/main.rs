#![no_std]
#![no_main]
use satrs::pus::verification::{
    FailParams, TcStateAccepted, VerificationReportCreator, VerificationToken,
};
use satrs::spacepackets::ecss::tc::PusTcReader;
use satrs::spacepackets::ecss::tm::{PusTmCreator, PusTmSecondaryHeader};
use satrs::spacepackets::ecss::EcssEnumU16;
use satrs::spacepackets::CcsdsPacket;
use satrs::spacepackets::{ByteConversionError, SpHeader};
// global logger + panicking-behavior + memory layout
use satrs_example_stm32f3_disco as _;

use rtic::app;

use heapless::{mpmc::Q8, Vec};
#[allow(unused_imports)]
use rtic_monotonics::systick::fugit::{MillisDurationU32, TimerInstantU32};
use rtic_monotonics::systick::ExtU32;
use satrs::seq_count::SequenceCountProviderCore;
use satrs::spacepackets::{ecss::PusPacket, ecss::WritablePusPacket};
use stm32f3xx_hal::dma::dma1;
use stm32f3xx_hal::gpio::{PushPull, AF7, PA2, PA3};
use stm32f3xx_hal::pac::USART2;
use stm32f3xx_hal::serial::{Rx, RxEvent, Serial, SerialDmaRx, SerialDmaTx, Tx, TxEvent};

const UART_BAUD: u32 = 115200;
const DEFAULT_BLINK_FREQ_MS: u32 = 1000;
const TX_HANDLER_FREQ_MS: u32 = 20;
const MIN_DELAY_BETWEEN_TX_PACKETS_MS: u32 = 5;
const MAX_TC_LEN: usize = 128;
const MAX_TM_LEN: usize = 128;
pub const PUS_APID: u16 = 0x02;

type TxType = Tx<USART2, PA2<AF7<PushPull>>>;
type RxType = Rx<USART2, PA3<AF7<PushPull>>>;
type InstantFugit = TimerInstantU32<1000>;
type TxDmaTransferType = SerialDmaTx<&'static [u8], dma1::C7, TxType>;
type RxDmaTransferType = SerialDmaRx<&'static mut [u8], dma1::C6, RxType>;

// This is the predictable maximum overhead of the COBS encoding scheme.
// It is simply the maximum packet lenght dividied by 254 rounded up.
const COBS_TC_OVERHEAD: usize = (MAX_TC_LEN + 254 - 1) / 254;
const COBS_TM_OVERHEAD: usize = (MAX_TM_LEN + 254 - 1) / 254;

const TC_BUF_LEN: usize = MAX_TC_LEN + COBS_TC_OVERHEAD;
const TM_BUF_LEN: usize = MAX_TC_LEN + COBS_TM_OVERHEAD;

// This is a static buffer which should ONLY (!) be used as the TX DMA
// transfer buffer.
static mut DMA_TX_BUF: [u8; TM_BUF_LEN] = [0; TM_BUF_LEN];
// This is a static buffer which should ONLY (!) be used as the RX DMA
// transfer buffer.
static mut DMA_RX_BUF: [u8; TC_BUF_LEN] = [0; TC_BUF_LEN];

type TmPacket = Vec<u8, MAX_TM_LEN>;
type TcPacket = Vec<u8, MAX_TC_LEN>;

static TM_REQUESTS: Q8<TmPacket> = Q8::new();

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

impl SequenceCountProviderCore<u16> for SeqCountProviderAtomicRef {
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
    tx: TxType,
    dma_channel: dma1::C7,
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

fn send_tm(tm_creator: PusTmCreator) -> Result<(), TmSendError> {
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
    TM_REQUESTS
        .enqueue(tm_vec)
        .map_err(|_| TmSendError::Queue)?;
    Ok(())
}

fn handle_tm_send_error(error: TmSendError) {
    defmt::warn!("sending tm failed with error {}", error);
}

pub enum UartTxState {
    // Wrapped in an option because we need an owned type later.
    Idle(Option<TxIdle>),
    // Same as above
    Transmitting(Option<TxDmaTransferType>),
}

pub struct UartTxShared {
    last_completed: Option<InstantFugit>,
    state: UartTxState,
}

pub struct RequestWithToken {
    token: VerificationToken<TcStateAccepted>,
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

pub fn convert_pus_tc_to_request(
    tc: &PusTcReader,
    verif_reporter: &mut VerificationReportCreator,
    src_data_buf: &mut [u8],
    timestamp: &[u8],
) -> Result<RequestWithToken, RequestError> {
    defmt::info!(
        "Found PUS TC [{},{}] with length {}",
        tc.service(),
        tc.subservice(),
        tc.len_packed()
    );

    let token = verif_reporter.add_tc(tc);
    if tc.apid() != PUS_APID {
        defmt::warn!("Received tc with unknown APID {}", tc.apid());
        let result = send_tm(
            verif_reporter
                .acceptance_failure(
                    src_data_buf,
                    token,
                    SEQ_COUNT_PROVIDER.get_and_increment(),
                    0,
                    FailParams::new(timestamp, &EcssEnumU16::new(0), &[]),
                )
                .unwrap(),
        );
        if let Err(e) = result {
            handle_tm_send_error(e);
        }
        return Err(RequestError::InvalidApid);
    }
    let (tm_creator, accepted_token) = verif_reporter
        .acceptance_success(
            src_data_buf,
            token,
            SEQ_COUNT_PROVIDER.get_and_increment(),
            0,
            timestamp,
        )
        .unwrap();

    if let Err(e) = send_tm(tm_creator) {
        handle_tm_send_error(e);
    }

    if tc.service() == 17 && tc.subservice() == 1 {
        if tc.subservice() == 1 {
            return Ok(RequestWithToken {
                request: Request::Ping,
                token: accepted_token,
            });
        } else {
            return Err(RequestError::InvalidSubservice);
        }
    } else if tc.service() == 8 {
        if tc.subservice() == 1 {
            if tc.user_data().len() < 4 {
                return Err(RequestError::NotEnoughAppData);
            }
            let new_freq_ms = u32::from_be_bytes(tc.user_data()[0..4].try_into().unwrap());
            return Ok(RequestWithToken {
                request: Request::ChangeBlinkFrequency(new_freq_ms),
                token: accepted_token,
            });
        } else {
            return Err(RequestError::InvalidSubservice);
        }
    } else {
        return Err(RequestError::InvalidService);
    }
}

#[app(device = stm32f3xx_hal::pac, peripherals = true)]
mod app {
    use super::*;
    use core::slice::Iter;
    use rtic_monotonics::systick::Systick;
    use rtic_monotonics::Monotonic;
    use satrs::pus::verification::{TcStateStarted, VerificationReportCreator};
    use satrs::spacepackets::{ecss::tc::PusTcReader, time::cds::P_FIELD_BASE};
    #[allow(unused_imports)]
    use stm32f3_discovery::leds::Direction;
    use stm32f3_discovery::leds::Leds;
    use stm32f3xx_hal::prelude::*;

    use stm32f3_discovery::switch_hal::OutputSwitch;
    use stm32f3xx_hal::Switch;
    #[allow(dead_code)]
    type SerialType = Serial<USART2, (PA2<AF7<PushPull>>, PA3<AF7<PushPull>>)>;

    #[shared]
    struct Shared {
        blink_freq: MillisDurationU32,
        tx_shared: UartTxShared,
        rx_transfer: Option<RxDmaTransferType>,
    }

    #[local]
    struct Local {
        verif_reporter: VerificationReportCreator,
        leds: Leds,
        last_dir: Direction,
        curr_dir: Iter<'static, Direction>,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let mut rcc = cx.device.RCC.constrain();

        // Initialize the systick interrupt & obtain the token to prove that we did
        let systick_mono_token = rtic_monotonics::create_systick_token!();
        Systick::start(cx.core.SYST, 8_000_000, systick_mono_token);

        let mut flash = cx.device.FLASH.constrain();
        let clocks = rcc
            .cfgr
            .use_hse(8.MHz())
            .sysclk(8.MHz())
            .pclk1(8.MHz())
            .freeze(&mut flash.acr);

        // Set up monotonic timer.
        //let mono_timer = MonoTimer::new(cx.core.DWT, clocks, &mut cx.core.DCB);

        defmt::info!("Starting sat-rs demo application for the STM32F3-Discovery");
        let mut gpioe = cx.device.GPIOE.split(&mut rcc.ahb);

        let leds = Leds::new(
            gpioe.pe8,
            gpioe.pe9,
            gpioe.pe10,
            gpioe.pe11,
            gpioe.pe12,
            gpioe.pe13,
            gpioe.pe14,
            gpioe.pe15,
            &mut gpioe.moder,
            &mut gpioe.otyper,
        );
        let mut gpioa = cx.device.GPIOA.split(&mut rcc.ahb);
        // USART2 pins
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
        pins.1.internal_pull_up(&mut gpioa.pupdr, true);
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

        let dma1 = cx.device.DMA1.split(&mut rcc.ahb);
        let (mut tx_serial, mut rx_serial) = usart2.split();

        // This interrupt is immediately triggered, clear it. It will only be reset
        // by the hardware when data is received on RX (RXNE event)
        rx_serial.clear_event(RxEvent::Idle);
        // For some reason, this is also immediately triggered..
        tx_serial.clear_event(TxEvent::TransmissionComplete);
        let rx_transfer = rx_serial.read_exact(unsafe { DMA_RX_BUF.as_mut_slice() }, dma1.ch6);
        defmt::info!("Spawning tasks");
        blink::spawn().unwrap();
        serial_tx_handler::spawn().unwrap();

        let verif_reporter = VerificationReportCreator::new(PUS_APID).unwrap();

        (
            Shared {
                blink_freq: MillisDurationU32::from_ticks(DEFAULT_BLINK_FREQ_MS),
                tx_shared: UartTxShared {
                    last_completed: None,
                    state: UartTxState::Idle(Some(TxIdle {
                        tx: tx_serial,
                        dma_channel: dma1.ch7,
                    })),
                },
                rx_transfer: Some(rx_transfer),
            },
            Local {
                verif_reporter,
                leds,
                last_dir: Direction::North,
                curr_dir: Direction::iter(),
            },
        )
    }

    #[task(local = [leds, curr_dir, last_dir], shared=[blink_freq])]
    async fn blink(mut cx: blink::Context) {
        let blink::LocalResources {
            leds,
            curr_dir,
            last_dir,
            ..
        } = cx.local;
        let mut toggle_leds = |dir: &Direction| {
            let last_led = leds.for_direction(*last_dir);
            last_led.off().ok();
            let led = leds.for_direction(*dir);
            led.on().ok();
            *last_dir = *dir;
        };
        loop {
            match curr_dir.next() {
                Some(dir) => {
                    toggle_leds(dir);
                }
                None => {
                    *curr_dir = Direction::iter();
                    toggle_leds(curr_dir.next().unwrap());
                }
            }
            let current_blink_freq = cx.shared.blink_freq.lock(|current| *current);
            Systick::delay(current_blink_freq).await;
        }
    }

    #[task(
        shared = [tx_shared],
    )]
    async fn serial_tx_handler(mut cx: serial_tx_handler::Context) {
        loop {
            let is_idle = cx.shared.tx_shared.lock(|tx_shared| {
                if let UartTxState::Idle(_) = tx_shared.state {
                    return true;
                }
                false
            });
            if is_idle {
                let last_completed = cx.shared.tx_shared.lock(|shared| shared.last_completed);
                if let Some(last_completed) = last_completed {
                    let elapsed_ms = (Systick::now() - last_completed).to_millis();
                    if elapsed_ms < MIN_DELAY_BETWEEN_TX_PACKETS_MS {
                        Systick::delay((MIN_DELAY_BETWEEN_TX_PACKETS_MS - elapsed_ms).millis())
                            .await;
                    }
                }
            } else {
                // Check for completion after 1 ms
                Systick::delay(1.millis()).await;
                continue;
            }
            if let Some(vec) = TM_REQUESTS.dequeue() {
                cx.shared
                    .tx_shared
                    .lock(|tx_shared| match &mut tx_shared.state {
                        UartTxState::Idle(tx) => {
                            let encoded_len;
                            //debug!(target: "serial_tx_handler", "bytes: {:x?}", &buf[0..len]);
                            // Safety: We only copy the data into the TX DMA buffer in this task.
                            // If the DMA is active, another branch will be taken.
                            unsafe {
                                // 0 sentinel value as start marker
                                DMA_TX_BUF[0] = 0;
                                encoded_len =
                                    cobs::encode(&vec[0..vec.len()], &mut DMA_TX_BUF[1..]);
                                // Should never panic, we accounted for the overhead.
                                // Write into transfer buffer directly, no need for intermediate
                                // encoding buffer.
                                // 0 end marker
                                DMA_TX_BUF[encoded_len + 1] = 0;
                            }
                            //debug!(target: "serial_tx_handler", "Sending {} bytes", encoded_len + 2);
                            //debug!("sent: {:x?}", &mut_tx_dma_buf[0..encoded_len + 2]);
                            let tx_idle = tx.take().unwrap();
                            // Transfer completion and re-scheduling of new TX transfers will be done
                            // by the IRQ handler.
                            // SAFETY: The DMA is the exclusive writer to the DMA buffer now.
                            let transfer = tx_idle.tx.write_all(
                                unsafe { &DMA_TX_BUF[0..encoded_len + 2] },
                                tx_idle.dma_channel,
                            );
                            tx_shared.state = UartTxState::Transmitting(Some(transfer));
                            // The memory block is automatically returned to the pool when it is dropped.
                        }
                        UartTxState::Transmitting(_) => (),
                    });
                // Check for completion after 1 ms
                Systick::delay(1.millis()).await;
                continue;
            }
            // Nothing to do, and we are idle.
            Systick::delay(TX_HANDLER_FREQ_MS.millis()).await;
        }
    }

    #[task(
        local = [
            verif_reporter,
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
        match cobs::decode(&received_packet.as_slice()[start_idx..], decode_buf) {
            Ok(len) => {
                defmt::info!("Decoded packet length: {}", len);
                let pus_tc = PusTcReader::new(decode_buf);
                match pus_tc {
                    Ok((tc, _tc_len)) => {
                        match convert_pus_tc_to_request(
                            &tc,
                            cx.local.verif_reporter,
                            cx.local.src_data_buf,
                            cx.local.timestamp,
                        ) {
                            Ok(request_with_token) => {
                                let started_token = handle_start_verification(
                                    request_with_token.token,
                                    cx.local.verif_reporter,
                                    cx.local.src_data_buf,
                                    cx.local.timestamp,
                                );

                                match request_with_token.request {
                                    Request::Ping => {
                                        handle_ping_request(cx.local.timestamp);
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
                                    started_token,
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

    fn handle_ping_request(timestamp: &[u8]) {
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
        if TM_REQUESTS.enqueue(tm_packet).is_err() {
            defmt::warn!("TC queue full");
            return;
        }
    }

    fn handle_start_verification(
        accepted_token: VerificationToken<TcStateAccepted>,
        verif_reporter: &mut VerificationReportCreator,
        src_data_buf: &mut [u8],
        timestamp: &[u8],
    ) -> VerificationToken<TcStateStarted> {
        let (tm_creator, started_token) = verif_reporter
            .start_success(
                src_data_buf,
                accepted_token,
                SEQ_COUNT_PROVIDER.get(),
                0,
                &timestamp,
            )
            .unwrap();
        let result = send_tm(tm_creator);
        if let Err(e) = result {
            handle_tm_send_error(e);
        }
        started_token
    }

    fn handle_completion_verification(
        started_token: VerificationToken<TcStateStarted>,
        verif_reporter: &mut VerificationReportCreator,
        src_data_buf: &mut [u8],
        timestamp: &[u8],
    ) {
        let result = send_tm(
            verif_reporter
                .completion_success(
                    src_data_buf,
                    started_token,
                    SEQ_COUNT_PROVIDER.get(),
                    0,
                    timestamp,
                )
                .unwrap(),
        );
        if let Err(e) = result {
            handle_tm_send_error(e);
        }
    }

    #[task(binds = DMA1_CH6, shared = [rx_transfer])]
    fn rx_dma_isr(mut cx: rx_dma_isr::Context) {
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
    }

    #[task(binds = USART2_EXTI26, shared = [rx_transfer, tx_shared])]
    fn serial_isr(mut cx: serial_isr::Context) {
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
                        tx_shared.last_completed = Some(Systick::now());
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
    }
}
