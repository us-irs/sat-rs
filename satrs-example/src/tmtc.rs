use satrs_core::events::EventU32;
use satrs_core::hal::host::udp_server::{ReceiveResult, UdpTcServer};
use satrs_core::params::Params;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, SendError, Sender, TryRecvError};
use std::sync::{Arc, LockResult, Mutex};
use std::thread;
use std::time::Duration;

use crate::ccsds::CcsdsReceiver;
use crate::pus::PusReceiver;
use crate::requests::RequestWithToken;
use satrs_core::pool::{SharedPool, StoreAddr, StoreError};
use satrs_core::pus::event_man::EventRequestWithToken;
use satrs_core::pus::scheduling::PusScheduler;
use satrs_core::pus::verification::StdVerifReporterWithSender;
use satrs_core::spacepackets::{ecss::PusPacket, tc::PusTc, tm::PusTm, SpHeader};
use satrs_core::tmtc::{
    CcsdsDistributor, CcsdsError, PusServiceProvider, ReceivesCcsdsTc, ReceivesEcssPusTc,
};

pub const PUS_APID: u16 = 0x02;

pub struct OtherArgs {
    pub sock_addr: SocketAddr,
    pub verif_reporter: StdVerifReporterWithSender,
    pub event_sender: Sender<(EventU32, Option<Params>)>,
    pub event_request_tx: Sender<EventRequestWithToken>,
    pub request_map: HashMap<u32, Sender<RequestWithToken>>,
}

pub struct TmArgs {
    pub tm_store: TmStore,
    pub tm_sink_sender: Sender<StoreAddr>,
    pub tm_server_rx: Receiver<StoreAddr>,
}

pub struct TcArgs {
    pub tc_source: PusTcSource,
    pub tc_receiver: Receiver<StoreAddr>,
}

impl TcArgs {
    fn split(self) -> (PusTcSource, Receiver<StoreAddr>) {
        (self.tc_source, self.tc_receiver)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MpscStoreAndSendError {
    StoreError(StoreError),
    SendError(SendError<StoreAddr>),
}

impl Display for MpscStoreAndSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            MpscStoreAndSendError::StoreError(s) => {
                write!(f, "store error {}", s)
            }
            MpscStoreAndSendError::SendError(s) => {
                write!(f, "send error {}", s)
            }
        }
    }
}

impl Error for MpscStoreAndSendError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            MpscStoreAndSendError::StoreError(s) => Some(s),
            MpscStoreAndSendError::SendError(s) => Some(s),
        }
    }
}

impl From<StoreError> for MpscStoreAndSendError {
    fn from(value: StoreError) -> Self {
        Self::StoreError(value)
    }
}

impl From<SendError<StoreAddr>> for MpscStoreAndSendError {
    fn from(value: SendError<StoreAddr>) -> Self {
        Self::SendError(value)
    }
}

#[derive(Clone)]
pub struct TmStore {
    pub pool: SharedPool,
}

#[derive(Clone)]
pub struct TcStore {
    pub pool: SharedPool,
}

impl TmStore {
    pub fn add_pus_tm(&mut self, pus_tm: &PusTm) -> StoreAddr {
        let mut pg = self.pool.write().expect("error locking TM store");
        let (addr, buf) = pg.free_element(pus_tm.len_packed()).expect("Store error");
        pus_tm
            .write_to_bytes(buf)
            .expect("writing PUS TM to store failed");
        addr
    }
}

impl TcStore {
    pub fn add_pus_tc(&mut self, pus_tc: &PusTc) -> Result<StoreAddr, StoreError> {
        let mut pg = self.pool.write().expect("error locking TC store");
        let (addr, buf) = pg.free_element(pus_tc.len_packed())?;
        pus_tc
            .write_to_bytes(buf)
            .expect("writing PUS TC to store failed");
        Ok(addr)
    }
}

pub struct TmFunnel {
    pub tm_funnel_rx: Receiver<StoreAddr>,
    pub tm_server_tx: Sender<StoreAddr>,
}

pub struct UdpTmtcServer {
    udp_tc_server: UdpTcServer<CcsdsError<MpscStoreAndSendError>>,
    tm_rx: Receiver<StoreAddr>,
    tm_store: SharedPool,
}

#[derive(Clone)]
pub struct PusTcSource {
    pub tc_source: Sender<StoreAddr>,
    pub tc_store: TcStore,
}

impl ReceivesEcssPusTc for PusTcSource {
    type Error = MpscStoreAndSendError;

    fn pass_pus_tc(&mut self, _: &SpHeader, pus_tc: &PusTc) -> Result<(), Self::Error> {
        let addr = self.tc_store.add_pus_tc(pus_tc)?;
        self.tc_source.send(addr)?;
        Ok(())
    }
}

impl ReceivesCcsdsTc for PusTcSource {
    type Error = MpscStoreAndSendError;

    fn pass_ccsds(&mut self, _: &SpHeader, tc_raw: &[u8]) -> Result<(), Self::Error> {
        let mut pool = self.tc_store.pool.write().expect("locking pool failed");
        let addr = pool.add(tc_raw)?;
        drop(pool);
        self.tc_source.send(addr)?;
        Ok(())
    }
}
pub fn core_tmtc_task(args: OtherArgs, mut tc_args: TcArgs, tm_args: TmArgs) {
    let mut scheduler = Arc::new(Mutex::new(
        PusScheduler::new_with_current_init_time(Duration::from_secs(5)).unwrap(),
    ));
    let mut sched_clone = scheduler.clone();
    let mut pus_receiver = PusReceiver::new(
        PUS_APID,
        tm_args.tm_sink_sender,
        tm_args.tm_store.clone(),
        args.verif_reporter,
        tc_args.tc_source.clone(),
        args.event_request_tx,
        args.request_map,
        sched_clone,
    );
    let ccsds_receiver = CcsdsReceiver {
        tc_source: tc_args.tc_source.clone(),
    };
    let ccsds_distributor = CcsdsDistributor::new(Box::new(ccsds_receiver));
    let udp_tc_server = UdpTcServer::new(args.sock_addr, 2048, Box::new(ccsds_distributor))
        .expect("Creating UDP TMTC server failed");

    let mut udp_tmtc_server = UdpTmtcServer {
        udp_tc_server,
        tm_rx: tm_args.tm_server_rx,
        tm_store: tm_args.tm_store.pool.clone(),
    };

    let mut test_closure = |boolvar: bool, store_addr: &StoreAddr| true;
    let (mut tc_source, mut tc_receiver) = tc_args.split();

    loop {
        let mut tmtc_sched = scheduler.clone();
        core_tmtc_loop(
            &mut udp_tmtc_server,
            &mut tc_source,
            &mut tc_receiver,
            &mut pus_receiver,
            tmtc_sched,
        );
        thread::sleep(Duration::from_millis(400));
    }
}

fn core_tmtc_loop(
    udp_tmtc_server: &mut UdpTmtcServer,
    tc_source: &mut PusTcSource,
    tc_receiver: &mut Receiver<StoreAddr>,
    pus_receiver: &mut PusReceiver,
    scheduler: Arc<Mutex<PusScheduler>>,
) {
    let releaser = |enabled: bool, addr: &StoreAddr| {
        tc_source.tc_source.send(*addr);
        true
    };

    let mut scheduler = scheduler.lock().expect("Lock of scheduler failed");
    match tc_source.tc_store.pool.write() {
        Ok(mut pool) => match scheduler.release_telecommands(releaser, pool.as_mut()) {
            Ok(_) => {}
            Err(_) => {}
        },
        Err(_) => {}
    }

    while poll_tc_server(udp_tmtc_server) {}
    match tc_receiver.try_recv() {
        Ok(addr) => {
            let pool = tc_source
                .tc_store
                .pool
                .read()
                .expect("locking tc pool failed");
            let data = pool.read(&addr).expect("reading pool failed");
            match PusTc::from_bytes(data) {
                Ok((pus_tc, _)) => {
                    pus_receiver
                        .handle_pus_tc_packet(pus_tc.service(), pus_tc.sp_header(), &pus_tc)
                        .ok();
                }
                Err(e) => {
                    println!("error creating PUS TC from raw data: {}", e);
                    println!("raw data: {:x?}", data);
                }
            }
        }
        Err(e) => {
            if let TryRecvError::Disconnected = e {
                println!("tmtc thread: sender disconnected")
            }
        }
    }
    if let Some(recv_addr) = udp_tmtc_server.udp_tc_server.last_sender() {
        core_tm_handling(udp_tmtc_server, &recv_addr);
    }
}

fn poll_tc_server(udp_tmtc_server: &mut UdpTmtcServer) -> bool {
    match udp_tmtc_server.udp_tc_server.try_recv_tc() {
        Ok(_) => true,
        Err(e) => match e {
            ReceiveResult::ReceiverError(e) => match e {
                CcsdsError::PacketError(e) => {
                    println!("Got packet error: {e:?}");
                    true
                }
                CcsdsError::CustomError(_) => {
                    println!("Unknown receiver error");
                    true
                }
            },
            ReceiveResult::IoError(e) => {
                println!("IO error {e}");
                false
            }
            ReceiveResult::NothingReceived => false,
        },
    }
}

fn core_tm_handling(udp_tmtc_server: &mut UdpTmtcServer, recv_addr: &SocketAddr) {
    while let Ok(addr) = udp_tmtc_server.tm_rx.try_recv() {
        let mut store_lock = udp_tmtc_server
            .tm_store
            .write()
            .expect("locking TM store failed");
        let pg = store_lock.read_with_guard(addr);
        let buf = pg.read().expect("error reading TM pool data");
        if buf.len() > 9 {
            let service = buf[7];
            let subservice = buf[8];
            println!("Sending PUS TM[{},{}]", service, subservice)
        } else {
            println!("Sending PUS TM");
        }
        udp_tmtc_server
            .udp_tc_server
            .socket
            .send_to(buf, recv_addr)
            .expect("sending TM failed");
    }
}
