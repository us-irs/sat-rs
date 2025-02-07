#![no_main]
#![no_std]
extern crate alloc;

use rtic::app;
use rtic_monotonics::systick::prelude::*;
use satrs::pool::{PoolAddr, PoolProvider, StaticHeaplessMemoryPool};
use satrs::static_subpool;
// global logger + panicking-behavior + memory layout
use satrs_stm32h7_nucleo_rtic as _;
use smoltcp::socket::udp::UdpMetadata;
use smoltcp::socket::{dhcpv4, udp};

use core::mem::MaybeUninit;
use embedded_alloc::LlffHeap as Heap;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet, SocketStorage};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr};
use stm32h7xx_hal::ethernet;

const DEFAULT_BLINK_FREQ_MS: u32 = 1000;
const PORT: u16 = 7301;

const HEAP_SIZE: usize = 131_072;

const TC_SOURCE_CHANNEL_DEPTH: usize = 16;
pub type SharedPool = StaticHeaplessMemoryPool<3>;
pub type TcSourceChannel = rtic_sync::channel::Channel<PoolAddr, TC_SOURCE_CHANNEL_DEPTH>;
pub type TcSourceTx = rtic_sync::channel::Sender<'static, PoolAddr, TC_SOURCE_CHANNEL_DEPTH>;
pub type TcSourceRx = rtic_sync::channel::Receiver<'static, PoolAddr, TC_SOURCE_CHANNEL_DEPTH>;

#[global_allocator]
static HEAP: Heap = Heap::empty();

systick_monotonic!(Mono, 1000);

// We place the memory pool buffers inside the larger AXISRAM.
pub const SUBPOOL_SMALL_NUM_BLOCKS: u16 = 32;
pub const SUBPOOL_SMALL_BLOCK_SIZE: usize = 32;
pub const SUBPOOL_MEDIUM_NUM_BLOCKS: u16 = 16;
pub const SUBPOOL_MEDIUM_BLOCK_SIZE: usize = 128;
pub const SUBPOOL_LARGE_NUM_BLOCKS: u16 = 8;
pub const SUBPOOL_LARGE_BLOCK_SIZE: usize = 2048;

// This data will be held by Net through a mutable reference
pub struct NetStorageStatic<'a> {
    socket_storage: [SocketStorage<'a>; 8],
}
// MaybeUninit allows us write code that is correct even if STORE is not
// initialised by the runtime
static mut STORE: MaybeUninit<NetStorageStatic> = MaybeUninit::uninit();

static mut UDP_RX_META: [udp::PacketMetadata; 12] = [udp::PacketMetadata::EMPTY; 12];
static mut UDP_RX: [u8; 2048] = [0; 2048];
static mut UDP_TX_META: [udp::PacketMetadata; 12] = [udp::PacketMetadata::EMPTY; 12];
static mut UDP_TX: [u8; 2048] = [0; 2048];

/// Locally administered MAC address
const MAC_ADDRESS: [u8; 6] = [0x02, 0x00, 0x11, 0x22, 0x33, 0x44];

pub struct Net {
    iface: Interface,
    ethdev: ethernet::EthernetDMA<4, 4>,
    dhcp_handle: SocketHandle,
}

impl Net {
    pub fn new(
        sockets: &mut SocketSet<'static>,
        mut ethdev: ethernet::EthernetDMA<4, 4>,
        ethernet_addr: HardwareAddress,
    ) -> Self {
        let config = Config::new(ethernet_addr);
        let mut iface = Interface::new(
            config,
            &mut ethdev,
            smoltcp::time::Instant::from_millis(Mono::now().duration_since_epoch().to_millis()),
        );
        // Create sockets
        let dhcp_socket = dhcpv4::Socket::new();
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(IpAddress::v4(192, 168, 1, 99), 0));
        });

        let dhcp_handle = sockets.add(dhcp_socket);
        Net {
            iface,
            ethdev,
            dhcp_handle,
        }
    }

    /// Polls on the ethernet interface. You should refer to the smoltcp
    /// documentation for poll() to understand how to call poll efficiently
    pub fn poll<'a>(&mut self, sockets: &'a mut SocketSet) -> bool {
        let uptime = Mono::now().duration_since_epoch();
        let timestamp = smoltcp::time::Instant::from_millis(uptime.to_millis());

        self.iface.poll(timestamp, &mut self.ethdev, sockets)
    }

    pub fn poll_dhcp<'a>(&mut self, sockets: &'a mut SocketSet) -> Option<dhcpv4::Event<'a>> {
        let opt_event = sockets.get_mut::<dhcpv4::Socket>(self.dhcp_handle).poll();
        if let Some(event) = &opt_event {
            match event {
                dhcpv4::Event::Deconfigured => {
                    defmt::info!("DHCP lost configuration");
                    self.iface.update_ip_addrs(|addrs| addrs.clear());
                    self.iface.routes_mut().remove_default_ipv4_route();
                }
                dhcpv4::Event::Configured(config) => {
                    defmt::info!("DHCP configuration acquired");
                    defmt::info!("IP address: {}", config.address);
                    self.iface.update_ip_addrs(|addrs| {
                        addrs.clear();
                        addrs.push(IpCidr::Ipv4(config.address)).unwrap();
                    });

                    if let Some(router) = config.router {
                        defmt::debug!("Default gateway: {}", router);
                        self.iface
                            .routes_mut()
                            .add_default_ipv4_route(router)
                            .unwrap();
                    } else {
                        defmt::debug!("Default gateway: None");
                        self.iface.routes_mut().remove_default_ipv4_route();
                    }
                }
            }
        }
        opt_event
    }
}

pub struct UdpNet {
    udp_handle: SocketHandle,
    last_client: Option<UdpMetadata>,
    tc_source_tx: TcSourceTx,
}

impl UdpNet {
    pub fn new<'sockets>(sockets: &mut SocketSet<'sockets>, tc_source_tx: TcSourceTx) -> Self {
        // SAFETY: The RX and TX buffers are passed here and not used anywhere else.
        let udp_rx_buffer =
            smoltcp::socket::udp::PacketBuffer::new(unsafe { &mut UDP_RX_META[..] }, unsafe {
                &mut UDP_RX[..]
            });
        let udp_tx_buffer =
            smoltcp::socket::udp::PacketBuffer::new(unsafe { &mut UDP_TX_META[..] }, unsafe {
                &mut UDP_TX[..]
            });
        let udp_socket = smoltcp::socket::udp::Socket::new(udp_rx_buffer, udp_tx_buffer);

        let udp_handle = sockets.add(udp_socket);
        Self {
            udp_handle,
            last_client: None,
            tc_source_tx,
        }
    }

    pub fn poll<'sockets>(
        &mut self,
        sockets: &'sockets mut SocketSet,
        shared_pool: &mut SharedPool,
    ) {
        let socket = sockets.get_mut::<udp::Socket>(self.udp_handle);
        if !socket.is_open() {
            if let Err(e) = socket.bind(PORT) {
                defmt::warn!("binding UDP socket failed: {}", e);
            }
        }
        loop {
            match socket.recv() {
                Ok((data, client)) => {
                    match shared_pool.add(data) {
                        Ok(store_addr) => {
                            if let Err(e) = self.tc_source_tx.try_send(store_addr) {
                                defmt::warn!("TC source channel is full: {}", e);
                            }
                        }
                        Err(e) => {
                            defmt::warn!("could not add UDP packet to shared pool: {}", e);
                        }
                    }
                    self.last_client = Some(client);
                    // TODO: Implement packet wiretapping.
                }
                Err(e) => match e {
                    udp::RecvError::Exhausted => {
                        break;
                    }
                    udp::RecvError::Truncated => {
                        defmt::warn!("UDP packet was truncacted");
                    }
                },
            };
        }
    }
}

#[app(device = stm32h7xx_hal::stm32, peripherals = true)]
mod app {
    use core::ptr::addr_of_mut;

    use super::*;
    use rtic_monotonics::fugit::MillisDurationU32;
    use satrs::spacepackets::ecss::tc::PusTcReader;
    use stm32h7xx_hal::ethernet::{EthernetMAC, PHY};
    use stm32h7xx_hal::gpio::{Output, Pin};
    use stm32h7xx_hal::prelude::*;
    use stm32h7xx_hal::stm32::Interrupt;

    struct BlinkyLeds {
        led1: Pin<'B', 7, Output>,
        led2: Pin<'B', 14, Output>,
    }

    #[local]
    struct Local {
        leds: BlinkyLeds,
        link_led: Pin<'B', 0, Output>,
        net: Net,
        udp: UdpNet,
        tc_source_rx: TcSourceRx,
        phy: ethernet::phy::LAN8742A<EthernetMAC>,
    }

    #[shared]
    struct Shared {
        blink_freq: MillisDurationU32,
        eth_link_up: bool,
        sockets: SocketSet<'static>,
        shared_pool: SharedPool,
    }

    #[init]
    fn init(mut cx: init::Context) -> (Shared, Local) {
        defmt::println!("Starting sat-rs demo application for the STM32H743ZIT");

        let pwr = cx.device.PWR.constrain();
        let pwrcfg = pwr.freeze();

        let rcc = cx.device.RCC.constrain();
        // Try to keep the clock configuration similar to one used in STM examples:
        // https://github.com/STMicroelectronics/STM32CubeH7/blob/master/Projects/NUCLEO-H743ZI/Examples/GPIO/GPIO_EXTI/Src/main.c
        let ccdr = rcc
            .sys_ck(400.MHz())
            .hclk(200.MHz())
            .use_hse(8.MHz())
            .bypass_hse()
            .pclk1(100.MHz())
            .pclk2(100.MHz())
            .pclk3(100.MHz())
            .pclk4(100.MHz())
            .freeze(pwrcfg, &cx.device.SYSCFG);

        // Initialize the systick interrupt & obtain the token to prove that we did
        Mono::start(cx.core.SYST, ccdr.clocks.sys_ck().to_Hz());

        // Those are used in the smoltcp of the stm32h7xx-hal , I am not fully sure what they are
        // good for.
        cx.core.SCB.enable_icache();
        cx.core.DWT.enable_cycle_counter();

        let gpioa = cx.device.GPIOA.split(ccdr.peripheral.GPIOA);
        let gpiob = cx.device.GPIOB.split(ccdr.peripheral.GPIOB);
        let gpioc = cx.device.GPIOC.split(ccdr.peripheral.GPIOC);
        let gpiog = cx.device.GPIOG.split(ccdr.peripheral.GPIOG);

        let link_led = gpiob.pb0.into_push_pull_output();
        let mut led1 = gpiob.pb7.into_push_pull_output();
        let mut led2 = gpiob.pb14.into_push_pull_output();

        // Criss-cross pattern looks cooler.
        led1.set_high();
        led2.set_low();
        let leds = BlinkyLeds { led1, led2 };

        let rmii_ref_clk = gpioa.pa1.into_alternate::<11>();
        let rmii_mdio = gpioa.pa2.into_alternate::<11>();
        let rmii_mdc = gpioc.pc1.into_alternate::<11>();
        let rmii_crs_dv = gpioa.pa7.into_alternate::<11>();
        let rmii_rxd0 = gpioc.pc4.into_alternate::<11>();
        let rmii_rxd1 = gpioc.pc5.into_alternate::<11>();
        let rmii_tx_en = gpiog.pg11.into_alternate::<11>();
        let rmii_txd0 = gpiog.pg13.into_alternate::<11>();
        let rmii_txd1 = gpiob.pb13.into_alternate::<11>();

        let mac_addr = smoltcp::wire::EthernetAddress::from_bytes(&MAC_ADDRESS);

        /// Ethernet descriptor rings are a global singleton
        #[link_section = ".sram3.eth"]
        static mut DES_RING: MaybeUninit<ethernet::DesRing<4, 4>> = MaybeUninit::uninit();

        let (eth_dma, eth_mac) = ethernet::new(
            cx.device.ETHERNET_MAC,
            cx.device.ETHERNET_MTL,
            cx.device.ETHERNET_DMA,
            (
                rmii_ref_clk,
                rmii_mdio,
                rmii_mdc,
                rmii_crs_dv,
                rmii_rxd0,
                rmii_rxd1,
                rmii_tx_en,
                rmii_txd0,
                rmii_txd1,
            ),
            // SAFETY: We do not move the returned DMA struct across thread boundaries, so this
            // should be safe according to the docs.
            unsafe { DES_RING.assume_init_mut() },
            mac_addr,
            ccdr.peripheral.ETH1MAC,
            &ccdr.clocks,
        );
        // Initialise ethernet PHY...
        let mut lan8742a = ethernet::phy::LAN8742A::new(eth_mac.set_phy_addr(0));
        lan8742a.phy_reset();
        lan8742a.phy_init();

        unsafe {
            ethernet::enable_interrupt();
            cx.core.NVIC.set_priority(Interrupt::ETH, 196); // Mid prio
            cortex_m::peripheral::NVIC::unmask(Interrupt::ETH);
        }

        // unsafe: mutable reference to static storage, we only do this once
        let store = unsafe {
            let store_ptr = STORE.as_mut_ptr();

            // Initialise the socket_storage field. Using `write` instead of
            // assignment via `=` to not call `drop` on the old, uninitialised
            // value
            addr_of_mut!((*store_ptr).socket_storage).write([SocketStorage::EMPTY; 8]);

            // Now that all fields are initialised we can safely use
            // assume_init_mut to return a mutable reference to STORE
            STORE.assume_init_mut()
        };

        let (tc_source_tx, tc_source_rx) =
            rtic_sync::make_channel!(PoolAddr, TC_SOURCE_CHANNEL_DEPTH);

        let mut sockets = SocketSet::new(&mut store.socket_storage[..]);
        let net = Net::new(&mut sockets, eth_dma, mac_addr.into());
        let udp = UdpNet::new(&mut sockets, tc_source_tx);

        let mut shared_pool: SharedPool = StaticHeaplessMemoryPool::new(true);
        static_subpool!(
            SUBPOOL_SMALL,
            SUBPOOL_SMALL_SIZES,
            SUBPOOL_SMALL_NUM_BLOCKS as usize,
            SUBPOOL_SMALL_BLOCK_SIZE,
            link_section = ".axisram"
        );
        static_subpool!(
            SUBPOOL_MEDIUM,
            SUBPOOL_MEDIUM_SIZES,
            SUBPOOL_MEDIUM_NUM_BLOCKS as usize,
            SUBPOOL_MEDIUM_BLOCK_SIZE,
            link_section = ".axisram"
        );
        static_subpool!(
            SUBPOOL_LARGE,
            SUBPOOL_LARGE_SIZES,
            SUBPOOL_LARGE_NUM_BLOCKS as usize,
            SUBPOOL_LARGE_BLOCK_SIZE,
            link_section = ".axisram"
        );

        shared_pool
            .grow(
                SUBPOOL_SMALL.get_mut().unwrap(),
                SUBPOOL_SMALL_SIZES.get_mut().unwrap(),
                SUBPOOL_SMALL_NUM_BLOCKS,
                true,
            )
            .expect("growing heapless memory pool failed");
        shared_pool
            .grow(
                SUBPOOL_MEDIUM.get_mut().unwrap(),
                SUBPOOL_MEDIUM_SIZES.get_mut().unwrap(),
                SUBPOOL_MEDIUM_NUM_BLOCKS,
                true,
            )
            .expect("growing heapless memory pool failed");
        shared_pool
            .grow(
                SUBPOOL_LARGE.get_mut().unwrap(),
                SUBPOOL_LARGE_SIZES.get_mut().unwrap(),
                SUBPOOL_LARGE_NUM_BLOCKS,
                true,
            )
            .expect("growing heapless memory pool failed");

        // Set up global allocator. Use AXISRAM for the heap.
        #[link_section = ".axisram"]
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(&raw mut HEAP_MEM as usize, HEAP_SIZE) }

        eth_link_check::spawn().expect("eth link check failed");
        blinky::spawn().expect("spawning blink task failed");
        udp_task::spawn().expect("spawning UDP task failed");
        tc_source_task::spawn().expect("spawning TC source task failed");

        (
            Shared {
                blink_freq: MillisDurationU32::from_ticks(DEFAULT_BLINK_FREQ_MS),
                eth_link_up: false,
                sockets,
                shared_pool,
            },
            Local {
                link_led,
                leds,
                net,
                udp,
                tc_source_rx,
                phy: lan8742a,
            },
        )
    }

    #[task(local = [leds], shared=[blink_freq])]
    async fn blinky(mut cx: blinky::Context) {
        let leds = cx.local.leds;
        loop {
            leds.led1.toggle();
            leds.led2.toggle();
            let current_blink_freq = cx.shared.blink_freq.lock(|current| *current);
            Mono::delay(current_blink_freq).await;
        }
    }

    /// This task checks for the network link.
    #[task(local=[link_led, phy], shared=[eth_link_up])]
    async fn eth_link_check(mut cx: eth_link_check::Context) {
        let phy = cx.local.phy;
        let link_led = cx.local.link_led;
        loop {
            let link_was_up = cx.shared.eth_link_up.lock(|link_up| *link_up);
            if phy.poll_link() {
                if !link_was_up {
                    link_led.set_high();
                    cx.shared.eth_link_up.lock(|link_up| *link_up = true);
                    defmt::info!("Ethernet link up");
                }
            } else if link_was_up {
                link_led.set_low();
                cx.shared.eth_link_up.lock(|link_up| *link_up = false);
                defmt::info!("Ethernet link down");
            }
            Mono::delay(100.millis()).await;
        }
    }

    #[task(binds=ETH, local=[net], shared=[sockets])]
    fn eth_isr(mut cx: eth_isr::Context) {
        // SAFETY: We do not write the register mentioned inside the docs anywhere else.
        unsafe {
            ethernet::interrupt_handler();
        }
        // Check and process ETH frames and DHCP. UDP is checked in a different task.
        cx.shared.sockets.lock(|sockets| {
            cx.local.net.poll(sockets);
            cx.local.net.poll_dhcp(sockets);
        });
    }

    /// This task routes UDP packets.
    #[task(local=[udp], shared=[sockets, shared_pool])]
    async fn udp_task(mut cx: udp_task::Context) {
        loop {
            cx.shared.sockets.lock(|sockets| {
                cx.shared.shared_pool.lock(|pool| {
                    cx.local.udp.poll(sockets, pool);
                })
            });
            Mono::delay(40.millis()).await;
        }
    }

    /// This task handles all the incoming telecommands.
    #[task(local=[read_buf: [u8; 1024] = [0; 1024], tc_source_rx], shared=[shared_pool])]
    async fn tc_source_task(mut cx: tc_source_task::Context) {
        loop {
            let recv_result = cx.local.tc_source_rx.recv().await;
            match recv_result {
                Ok(pool_addr) => {
                    cx.shared.shared_pool.lock(|pool| {
                        match pool.read(&pool_addr, cx.local.read_buf.as_mut()) {
                            Ok(packet_len) => {
                                defmt::info!("received {} bytes in the TC source task", packet_len);
                                match PusTcReader::new(&cx.local.read_buf[0..packet_len]) {
                                    Ok((packet, _tc_len)) => {
                                        // TODO: Handle packet here or dispatch to dedicated PUS
                                        // handler? Dispatching could simplify some things and make
                                        // the software more scalable..
                                        defmt::info!("received PUS packet: {}", packet);
                                    }
                                    Err(e) => {
                                        defmt::info!("invalid TC format, not a PUS packet: {}", e);
                                    }
                                }
                                if let Err(e) = pool.delete(pool_addr) {
                                    defmt::warn!("deleting TC data failed: {}", e);
                                }
                            }
                            Err(e) => {
                                defmt::warn!("TC packet read failed: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    defmt::warn!("TC source reception error: {}", e);
                }
            };
        }
    }
}
