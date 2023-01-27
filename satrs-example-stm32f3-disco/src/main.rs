#![no_std]
#![no_main]
extern crate panic_itm;

use rtic::app;

use heapless::{
    mpmc::Q16,
    pool,
    pool::singleton::{Box, Pool},
};
#[allow(unused_imports)]
use itm_logger::{debug, info, logger_init, warn};
use satrs_core::spacepackets::{ecss::PusPacket, tm::PusTm};
use satrs_core::{
    pus::{EcssTmErrorWithSend, EcssTmSenderCore},
    seq_count::SequenceCountProviderCore,
};
use stm32f3xx_hal::dma::dma1;
use stm32f3xx_hal::gpio::{PushPull, AF7, PA2, PA3};
use stm32f3xx_hal::pac::USART2;
use stm32f3xx_hal::serial::{Rx, RxEvent, Serial, SerialDmaRx, SerialDmaTx, Tx, TxEvent};
use systick_monotonic::{fugit::Duration, Systick};

const UART_BAUD: u32 = 115200;
const BLINK_FREQ_MS: u64 = 1000;
const TX_HANDLER_FREQ_MS: u64 = 20;
const MIN_DELAY_BETWEEN_TX_PACKETS_MS: u16 = 5;
const MAX_TC_LEN: usize = 200;
const MAX_TM_LEN: usize = 200;
pub const PUS_APID: u16 = 0x02;

type TxType = Tx<USART2, PA2<AF7<PushPull>>>;
type RxType = Rx<USART2, PA3<AF7<PushPull>>>;
type MsDuration = Duration<u64, 1, 1000>;
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

static TX_REQUESTS: Q16<(Box<poolmod::TM>, usize)> = Q16::new();

const TC_POOL_SLOTS: usize = 12;
const TM_POOL_SLOTS: usize = 12;
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

// Otherwise, warnings because of heapless pool macro.
#[allow(non_camel_case_types)]
mod poolmod {
    use super::*;
    // Must hold full TC length including COBS overhead.
    pool!(TC: [u8; TC_BUF_LEN]);
    // Only encoded at the end, so no need to account for COBS overhead.
    pool!(TM: [u8; MAX_TM_LEN]);
}

pub struct TxIdle {
    tx: TxType,
    dma_channel: dma1::C7,
}

#[derive(Debug)]
pub enum TmStoreError {
    StoreFull,
    StoreSlotsTooSmall,
}

impl From<TmStoreError> for EcssTmErrorWithSend<TmStoreError> {
    fn from(value: TmStoreError) -> Self {
        Self::SendError(value)
    }
}

pub struct TmSender {
    mem_block: Option<Box<poolmod::TM>>,
    ctx: &'static str,
}

impl TmSender {
    pub fn new(mem_block: Box<poolmod::TM>, ctx: &'static str) -> Self {
        Self {
            mem_block: Some(mem_block),
            ctx,
        }
    }
}

impl EcssTmSenderCore for TmSender {
    type Error = TmStoreError;

    fn send_tm(
        &mut self,
        tm: PusTm,
    ) -> Result<(), satrs_core::pus::EcssTmErrorWithSend<Self::Error>> {
        let mem_block = self.mem_block.take();
        if mem_block.is_none() {
            panic!("send_tm should only be called once");
        }
        let mut mem_block = mem_block.unwrap();
        if tm.len_packed() > MAX_TM_LEN {
            return Err(EcssTmErrorWithSend::SendError(
                TmStoreError::StoreSlotsTooSmall,
            ));
        }
        tm.write_to_bytes(mem_block.as_mut_slice())
            .map_err(|e| EcssTmErrorWithSend::EcssTmError(e.into()))?;
        info!(target: self.ctx, "Sending TM[{},{}] with size {}", tm.service(), tm.subservice(), tm.len_packed());
        TX_REQUESTS
            .enqueue((mem_block, tm.len_packed()))
            .map_err(|_| TmStoreError::StoreFull)?;
        Ok(())
    }
}

pub enum UartTxState {
    // Wrapped in an option because we need an owned type later.
    Idle(Option<TxIdle>),
    // Same as above
    Transmitting(Option<TxDmaTransferType>),
}

#[app(device = stm32f3xx_hal::pac, peripherals = true, dispatchers = [TIM20_BRK, TIM20_UP, TIM20_TRG_COM])]
mod app {
    use super::*;
    use core::slice::Iter;
    use cortex_m::iprintln;
    use satrs_core::pus::verification::FailParams;
    use satrs_core::pus::verification::VerificationReporterCore;
    use satrs_core::spacepackets::{
        ecss::EcssEnumU16,
        tc::PusTc,
        time::cds::P_FIELD_BASE,
        tm::{PusTm, PusTmSecondaryHeader},
        CcsdsPacket, SpHeader,
    };
    #[allow(unused_imports)]
    use stm32f3_discovery::leds::Direction;
    use stm32f3_discovery::leds::Leds;
    use stm32f3xx_hal::prelude::*;
    use stm32f3xx_hal::Toggle;

    use stm32f3_discovery::switch_hal::OutputSwitch;
    #[allow(dead_code)]
    type SerialType = Serial<USART2, (PA2<AF7<PushPull>>, PA3<AF7<PushPull>>)>;

    #[shared]
    struct Shared {
        tx_transfer: UartTxState,
        rx_transfer: Option<RxDmaTransferType>,
    }

    #[local]
    struct Local {
        leds: Leds,
        last_dir: Direction,
        verif_reporter: VerificationReporterCore,
        curr_dir: Iter<'static, Direction>,
    }

    #[monotonic(binds = SysTick, default = true)]
    type MonoTimer = Systick<1000>;

    #[init(local = [
        tc_pool_mem: [u8; TC_BUF_LEN * TC_POOL_SLOTS] = [0; TC_BUF_LEN * TC_POOL_SLOTS],
        tm_pool_mem: [u8; MAX_TM_LEN * TM_POOL_SLOTS] = [0; MAX_TM_LEN * TM_POOL_SLOTS]
    ])]
    fn init(mut cx: init::Context) -> (Shared, Local, init::Monotonics) {
        let mut rcc = cx.device.RCC.constrain();

        let mono = Systick::new(cx.core.SYST, 8_000_000);
        logger_init();
        let mut flash = cx.device.FLASH.constrain();
        let clocks = rcc
            .cfgr
            .use_hse(8.MHz())
            .sysclk(8.MHz())
            .pclk1(8.MHz())
            .freeze(&mut flash.acr);
        // setup ITM output
        iprintln!(
            &mut cx.core.ITM.stim[0],
            "Starting sat-rs demo application for the STM32F3-Discovery"
        );
        let mut gpioe = cx.device.GPIOE.split(&mut rcc.ahb);
        // Assign memory to the pools.
        poolmod::TC::grow(cx.local.tc_pool_mem);
        poolmod::TM::grow(cx.local.tm_pool_mem);

        let verif_reporter = VerificationReporterCore::new(PUS_APID).unwrap();

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
        usart2.configure_rx_interrupt(RxEvent::Idle, Toggle::On);
        // This interrupt is enabled to re-schedule new transfers in the interrupt handler immediately.
        usart2.configure_tx_interrupt(TxEvent::TransmissionComplete, Toggle::On);

        let dma1 = cx.device.DMA1.split(&mut rcc.ahb);
        let (tx_serial, mut rx_serial) = usart2.split();

        // This interrupt is immediately triggered, clear it. It will only be reset
        // by the hardware when data is received on RX (RXNE event)
        rx_serial.clear_event(RxEvent::Idle);
        let rx_transfer = rx_serial.read_exact(unsafe { DMA_RX_BUF.as_mut_slice() }, dma1.ch6);
        info!(target: "init", "Spawning tasks");
        blink::spawn().unwrap();
        serial_tx_handler::spawn().unwrap();
        (
            Shared {
                tx_transfer: UartTxState::Idle(Some(TxIdle {
                    tx: tx_serial,
                    dma_channel: dma1.ch7,
                })),
                rx_transfer: Some(rx_transfer),
            },
            Local {
                leds,
                last_dir: Direction::North,
                curr_dir: Direction::iter(),
                verif_reporter,
            },
            init::Monotonics(mono),
        )
    }

    #[task(local = [leds, curr_dir, last_dir])]
    fn blink(cx: blink::Context) {
        let toggle_leds = |dir: &Direction| {
            let leds = cx.local.leds;
            let last_led = leds.for_direction(*cx.local.last_dir);
            last_led.off().ok();
            let led = leds.for_direction(*dir);
            led.on().ok();
            *cx.local.last_dir = *dir;
        };

        match cx.local.curr_dir.next() {
            Some(dir) => {
                toggle_leds(dir);
            }
            None => {
                *cx.local.curr_dir = Direction::iter();
                toggle_leds(cx.local.curr_dir.next().unwrap());
            }
        }
        blink::spawn_after(MsDuration::from_ticks(BLINK_FREQ_MS)).unwrap();
    }

    #[task(
        shared = [tx_transfer],
        local = []
    )]
    fn serial_tx_handler(mut cx: serial_tx_handler::Context) {
        if let Some((buf, len)) = TX_REQUESTS.dequeue() {
            cx.shared.tx_transfer.lock(|tx_state| match tx_state {
                UartTxState::Idle(tx) => {
                    //debug!(target: "serial_tx_handler", "bytes: {:x?}", &buf[0..len]);
                    // Safety: We only copy the data into the TX DMA buffer in this task.
                    // If the DMA is active, another branch will be taken.
                    let mut_tx_dma_buf = unsafe { &mut DMA_TX_BUF };
                    // 0 sentinel value as start marker
                    mut_tx_dma_buf[0] = 0;
                    // Should never panic, we accounted for the overhead.
                    // Write into transfer buffer directly, no need for intermediate
                    // encoding buffer.
                    let encoded_len = cobs::encode(&buf[0..len], &mut mut_tx_dma_buf[1..]);
                    // 0 end marker
                    mut_tx_dma_buf[encoded_len + 1] = 0;
                    //debug!(target: "serial_tx_handler", "Sending {} bytes", encoded_len + 2);
                    //debug!("sent: {:x?}", &mut_tx_dma_buf[0..encoded_len + 2]);
                    let tx_idle = tx.take().unwrap();
                    // Transfer completion and re-scheduling of new TX transfers will be done
                    // by the IRQ handler.
                    let transfer = tx_idle
                        .tx
                        .write_all(&mut_tx_dma_buf[0..encoded_len + 2], tx_idle.dma_channel);
                    *tx_state = UartTxState::Transmitting(Some(transfer));
                    // The memory block is automatically returned to the pool when it is dropped.
                }
                UartTxState::Transmitting(_) => {
                    // This is a SW configuration error. Only the ISR which
                    // detects transfer completion should be able to spawn a new
                    // task, and that ISR should set the state to IDLE.
                    panic!("invalid internal tx state detected")
                }
            })
        } else {
            cx.shared.tx_transfer.lock(|tx_state| {
                if let UartTxState::Idle(_) = tx_state {
                    serial_tx_handler::spawn_after(MsDuration::from_ticks(TX_HANDLER_FREQ_MS))
                        .unwrap();
                }
            });
        }
    }

    #[task(
        local = [
            stamp_buf: [u8; 7] = [0; 7],
            decode_buf: [u8; MAX_TC_LEN] = [0; MAX_TC_LEN],
            src_data_buf: [u8; MAX_TM_LEN] = [0; MAX_TM_LEN],
            verif_reporter
        ],
    )]
    fn serial_rx_handler(
        cx: serial_rx_handler::Context,
        received_packet: Box<poolmod::TC>,
        rx_len: usize,
    ) {
        let tgt: &'static str = "serial_rx_handler";
        cx.local.stamp_buf[0] = P_FIELD_BASE;
        info!(target: tgt, "Received packet with {} bytes", rx_len);
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
            warn!(
                target: tgt,
                "decoding error, can only process cobs encoded frames, data is all 0"
            );
            return;
        }
        let start_idx = start_idx.unwrap();
        match cobs::decode(&received_packet.as_slice()[start_idx..], decode_buf) {
            Ok(len) => {
                info!(target: tgt, "Decoded packet length: {}", len);
                let pus_tc = PusTc::from_bytes(decode_buf);
                let verif_reporter = cx.local.verif_reporter;
                match pus_tc {
                    Ok((tc, tc_len)) => handle_tc(
                        tc,
                        tc_len,
                        verif_reporter,
                        cx.local.src_data_buf,
                        cx.local.stamp_buf,
                        tgt,
                    ),
                    Err(e) => {
                        warn!(target: tgt, "Error unpacking PUS TC: {}", e);
                    }
                }
            }
            Err(_) => {
                warn!(
                    target: tgt,
                    "decoding error, can only process cobs encoded frames"
                )
            }
        }
    }

    fn handle_tc(
        tc: PusTc,
        tc_len: usize,
        verif_reporter: &mut VerificationReporterCore,
        src_data_buf: &mut [u8; MAX_TM_LEN],
        stamp_buf: &[u8; 7],
        tgt: &'static str,
    ) {
        info!(
            target: tgt,
            "Found PUS TC [{},{}] with length {}",
            tc.service(),
            tc.subservice(),
            tc_len
        );

        let token = verif_reporter.add_tc(&tc);
        if tc.apid() != PUS_APID {
            warn!(target: tgt, "Received tc with unknown APID {}", tc.apid());
            let sendable = verif_reporter
                .acceptance_failure(
                    src_data_buf,
                    token,
                    &SEQ_COUNT_PROVIDER,
                    FailParams::new(stamp_buf, &EcssEnumU16::new(0), None),
                )
                .unwrap();
            let mem_block = poolmod::TM::alloc().unwrap().init([0u8; MAX_TM_LEN]);
            let mut sender = TmSender::new(mem_block, tgt);
            if let Err(e) =
                verif_reporter.send_acceptance_failure(sendable, &SEQ_COUNT_PROVIDER, &mut sender)
            {
                warn!(target: tgt, "Sending acceptance failure failed: {:?}", e.0);
            };
            return;
        }
        let sendable = verif_reporter
            .acceptance_success(src_data_buf, token, &SEQ_COUNT_PROVIDER, stamp_buf)
            .unwrap();

        let mem_block = poolmod::TM::alloc().unwrap().init([0u8; MAX_TM_LEN]);
        let mut sender = TmSender::new(mem_block, tgt);
        let accepted_token = match verif_reporter.send_acceptance_success(
            sendable,
            &SEQ_COUNT_PROVIDER,
            &mut sender,
        ) {
            Ok(token) => token,
            Err(e) => {
                warn!(target: "serial_rx_handler", "Sending acceptance success failed: {:?}", e.0);
                return;
            }
        };

        if tc.service() == 17 {
            if tc.subservice() == 1 {
                let sendable = verif_reporter
                    .start_success(src_data_buf, accepted_token, &SEQ_COUNT_PROVIDER, stamp_buf)
                    .unwrap();
                let mem_block = poolmod::TM::alloc().unwrap().init([0u8; MAX_TM_LEN]);
                let mut sender = TmSender::new(mem_block, tgt);
                let started_token = match verif_reporter.send_start_success(
                    sendable,
                    &SEQ_COUNT_PROVIDER,
                    &mut sender,
                ) {
                    Ok(token) => token,
                    Err(e) => {
                        warn!(target: tgt, "Sending acceptance success failed: {:?}", e.0);
                        return;
                    }
                };
                info!(
                    target: tgt,
                    "Received PUS ping telecommand, sending ping reply TM[17,2]"
                );
                let mut sp_header =
                    SpHeader::tc_unseg(PUS_APID, SEQ_COUNT_PROVIDER.get(), 0).unwrap();
                let sec_header = PusTmSecondaryHeader::new_simple(17, 2, stamp_buf);
                let ping_reply = PusTm::new(&mut sp_header, sec_header, None, true);
                let mut mem_block = poolmod::TM::alloc().unwrap().init([0u8; MAX_TM_LEN]);
                let reply_len = ping_reply.write_to_bytes(mem_block.as_mut_slice()).unwrap();
                if TX_REQUESTS.enqueue((mem_block, reply_len)).is_err() {
                    warn!(target: tgt, "TC queue full");
                    return;
                }
                SEQ_COUNT_PROVIDER.increment();
                let sendable = verif_reporter
                    .completion_success(src_data_buf, started_token, &SEQ_COUNT_PROVIDER, stamp_buf)
                    .unwrap();
                let mem_block = poolmod::TM::alloc().unwrap().init([0u8; MAX_TM_LEN]);
                let mut sender = TmSender::new(mem_block, tgt);
                if let Err(e) = verif_reporter.send_step_or_completion_success(
                    sendable,
                    &SEQ_COUNT_PROVIDER,
                    &mut sender,
                ) {
                    warn!(target: tgt, "Sending completion success failed: {:?}", e.0);
                }
            } else {
                // TODO: Invalid subservice
            }
        }
    }

    #[task(binds = DMA1_CH6, shared = [rx_transfer])]
    fn rx_dma_isr(mut cx: rx_dma_isr::Context) {
        cx.shared.rx_transfer.lock(|rx_transfer| {
            let rx_ref = rx_transfer.as_ref().unwrap();
            if rx_ref.is_complete() {
                let uart_rx_owned = rx_transfer.take().unwrap();
                let (buf, c, rx) = uart_rx_owned.stop();
                // The received data is transferred to another task now to avoid any processing overhead
                // during the interrupt. There are multiple ways to do this, we use a memory pool here
                // to do this.
                let mut mem_block = poolmod::TC::alloc()
                    .expect("allocating memory block for rx failed")
                    .init([0u8; TC_BUF_LEN]);
                // Copy data into memory pool.
                mem_block.copy_from_slice(buf);
                *rx_transfer = Some(rx.read_exact(buf, c));
                // Only send owning pointer to pool memory and the received packet length.
                serial_rx_handler::spawn(mem_block, TC_BUF_LEN)
                    .expect("spawning rx handler task failed");
                // If this happens, there is a high chance that the maximum packet length was
                // exceeded. Circular mode is not used here, so data might be missed.
                warn!(
                    "rx transfer with maximum length {}, might miss data",
                    TC_BUF_LEN
                );
            }
        });
    }

    #[task(binds = USART2_EXTI26, shared = [rx_transfer, tx_transfer])]
    fn serial_isr(mut cx: serial_isr::Context) {
        cx.shared.tx_transfer.lock(|tx_state| match tx_state {
            UartTxState::Idle(_) => (),
            UartTxState::Transmitting(transfer) => {
                let transfer_ref = transfer.as_ref().unwrap();
                if transfer_ref.is_complete() {
                    let transfer = transfer.take().unwrap();
                    let (_, dma_channel, tx) = transfer.stop();
                    *tx_state = UartTxState::Idle(Some(TxIdle { tx, dma_channel }));
                    serial_tx_handler::spawn_after(MsDuration::from_ticks(
                        MIN_DELAY_BETWEEN_TX_PACKETS_MS.into(),
                    ))
                    .unwrap();
                }
            }
        });
        cx.shared.rx_transfer.lock(|rx_transfer| {
            let rx_transfer_ref = rx_transfer.as_ref().unwrap();
            // Received a partial packet.
            if rx_transfer_ref.is_event_triggered(RxEvent::Idle) {
                let rx_transfer_owned = rx_transfer.take().unwrap();
                let (buf, ch, mut rx, rx_len) = rx_transfer_owned.stop_and_return_received_bytes();
                // The received data is transferred to another task now to avoid any processing overhead
                // during the interrupt. There are multiple ways to do this, we use a memory pool here
                // to do this.
                let mut mem_block = poolmod::TC::alloc()
                    .expect("allocating memory block for rx failed")
                    .init([0u8; TC_BUF_LEN]);
                // Copy data into memory pool.
                mem_block[0..rx_len as usize].copy_from_slice(&buf[0..rx_len as usize]);
                rx.clear_event(RxEvent::Idle);
                // Only send owning pointer to pool memory and the received packet length.
                serial_rx_handler::spawn(mem_block, rx_len as usize)
                    .expect("spawning rx handler task failed");
                *rx_transfer = Some(rx.read_exact(buf, ch));
            }
        });
    }
}
