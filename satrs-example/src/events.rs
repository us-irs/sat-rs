use std::sync::mpsc::{self};

use crate::pus::create_verification_reporter;
use satrs::event_man::{EventMessageU32, EventRoutingError};
use satrs::params::WritableToBeBytes;
use satrs::pus::event::EventTmHookProvider;
use satrs::pus::verification::VerificationReporter;
use satrs::pus::EcssTmSenderCore;
use satrs::request::UniqueApidTargetId;
use satrs::{
    event_man::{
        EventManagerWithBoundedMpsc, EventSendProvider, EventU32SenderMpscBounded,
        MpscEventReceiver,
    },
    pus::{
        event_man::{
            DefaultPusEventU32Dispatcher, EventReporter, EventRequest, EventRequestWithToken,
        },
        verification::{TcStateStarted, VerificationReportingProvider, VerificationToken},
    },
    spacepackets::time::cds::CdsTime,
};
use satrs_example::config::components::PUS_EVENT_MANAGEMENT;

use crate::update_time;

// This helper sets the APID of the event sender for the PUS telemetry.
#[derive(Default)]
pub struct EventApidSetter {
    pub next_apid: u16,
}

impl EventTmHookProvider for EventApidSetter {
    fn modify_tm(&self, tm: &mut satrs::spacepackets::ecss::tm::PusTmCreator) {
        tm.set_apid(self.next_apid);
    }
}

/// The PUS event handler subscribes for all events and converts them into ECSS PUS 5 event
/// packets. It also handles the verification completion of PUS event service requests.
pub struct PusEventHandler<TmSender: EcssTmSenderCore> {
    event_request_rx: mpsc::Receiver<EventRequestWithToken>,
    pus_event_dispatcher: DefaultPusEventU32Dispatcher<()>,
    pus_event_man_rx: mpsc::Receiver<EventMessageU32>,
    tm_sender: TmSender,
    time_provider: CdsTime,
    timestamp: [u8; 7],
    verif_handler: VerificationReporter,
    event_apid_setter: EventApidSetter,
}

impl<TmSender: EcssTmSenderCore> PusEventHandler<TmSender> {
    pub fn new(
        tm_sender: TmSender,
        verif_handler: VerificationReporter,
        event_manager: &mut EventManagerWithBoundedMpsc,
        event_request_rx: mpsc::Receiver<EventRequestWithToken>,
    ) -> Self {
        let event_queue_cap = 30;
        let (pus_event_man_tx, pus_event_man_rx) = mpsc::sync_channel(event_queue_cap);

        // All events sent to the manager are routed to the PUS event manager, which generates PUS event
        // telemetry for each event.
        let event_reporter = EventReporter::new(PUS_EVENT_MANAGEMENT.raw(), 0, 0, 128).unwrap();
        let pus_event_dispatcher =
            DefaultPusEventU32Dispatcher::new_with_default_backend(event_reporter);
        let pus_event_man_send_provider = EventU32SenderMpscBounded::new(
            PUS_EVENT_MANAGEMENT.raw(),
            pus_event_man_tx,
            event_queue_cap,
        );

        event_manager.subscribe_all(pus_event_man_send_provider.target_id());
        event_manager.add_sender(pus_event_man_send_provider);

        Self {
            event_request_rx,
            pus_event_dispatcher,
            pus_event_man_rx,
            time_provider: CdsTime::new_with_u16_days(0, 0),
            timestamp: [0; 7],
            verif_handler,
            tm_sender,
            event_apid_setter: EventApidSetter::default(),
        }
    }

    pub fn handle_event_requests(&mut self) {
        let report_completion = |event_req: EventRequestWithToken, timestamp: &[u8]| {
            let started_token: VerificationToken<TcStateStarted> = event_req
                .token
                .try_into()
                .expect("expected start verification token");
            self.verif_handler
                .completion_success(&self.tm_sender, started_token, timestamp)
                .expect("Sending completion success failed");
        };
        // handle event requests
        if let Ok(event_req) = self.event_request_rx.try_recv() {
            match event_req.request {
                EventRequest::Enable(event) => {
                    self.pus_event_dispatcher
                        .enable_tm_for_event(&event)
                        .expect("Enabling TM failed");
                    update_time(&mut self.time_provider, &mut self.timestamp);
                    report_completion(event_req, &self.timestamp);
                }
                EventRequest::Disable(event) => {
                    self.pus_event_dispatcher
                        .disable_tm_for_event(&event)
                        .expect("Disabling TM failed");
                    update_time(&mut self.time_provider, &mut self.timestamp);
                    report_completion(event_req, &self.timestamp);
                }
            }
        }
    }

    pub fn generate_pus_event_tm(&mut self) {
        // Perform the generation of PUS event packets
        if let Ok(event_msg) = self.pus_event_man_rx.try_recv() {
            update_time(&mut self.time_provider, &mut self.timestamp);
            let param_vec = event_msg.params().map_or(Vec::new(), |param| {
                param.to_vec().expect("failed to convert params to vec")
            });
            self.event_apid_setter.next_apid = UniqueApidTargetId::from(event_msg.sender_id()).apid;
            self.pus_event_dispatcher
                .generate_pus_event_tm_generic(
                    &self.tm_sender,
                    &self.timestamp,
                    event_msg.event(),
                    Some(&param_vec),
                )
                .expect("Sending TM as event failed");
        }
    }
}

/// This is a thin wrapper around the event manager which also caches the sender component
/// used to send events to the event manager.
pub struct EventManagerWrapper {
    event_manager: EventManagerWithBoundedMpsc,
    event_sender: mpsc::Sender<EventMessageU32>,
}

impl EventManagerWrapper {
    pub fn new() -> Self {
        // The sender handle is the primary sender handle for all components which want to create events.
        // The event manager will receive the RX handle to receive all the events.
        let (event_sender, event_man_rx) = mpsc::channel();
        let event_recv = MpscEventReceiver::new(event_man_rx);
        Self {
            event_manager: EventManagerWithBoundedMpsc::new(event_recv),
            event_sender,
        }
    }

    // Returns a cached event sender to send events to the event manager for routing.
    pub fn clone_event_sender(&self) -> mpsc::Sender<EventMessageU32> {
        self.event_sender.clone()
    }

    pub fn event_manager(&mut self) -> &mut EventManagerWithBoundedMpsc {
        &mut self.event_manager
    }

    pub fn try_event_routing(&mut self) {
        let error_handler = |event_msg: &EventMessageU32, error: EventRoutingError| {
            self.routing_error_handler(event_msg, error)
        };
        // Perform the event routing.
        self.event_manager.try_event_handling(error_handler);
    }

    pub fn routing_error_handler(&self, event_msg: &EventMessageU32, error: EventRoutingError) {
        log::warn!("event routing error for event {event_msg:?}: {error:?}");
    }
}

pub struct EventHandler<TmSender: EcssTmSenderCore> {
    pub event_man_wrapper: EventManagerWrapper,
    pub pus_event_handler: PusEventHandler<TmSender>,
}

impl<TmSender: EcssTmSenderCore> EventHandler<TmSender> {
    pub fn new(
        tm_sender: TmSender,
        event_request_rx: mpsc::Receiver<EventRequestWithToken>,
    ) -> Self {
        let mut event_man_wrapper = EventManagerWrapper::new();
        let pus_event_handler = PusEventHandler::new(
            tm_sender,
            create_verification_reporter(PUS_EVENT_MANAGEMENT.id(), PUS_EVENT_MANAGEMENT.apid),
            event_man_wrapper.event_manager(),
            event_request_rx,
        );
        Self {
            event_man_wrapper,
            pus_event_handler,
        }
    }

    pub fn clone_event_sender(&self) -> mpsc::Sender<EventMessageU32> {
        self.event_man_wrapper.clone_event_sender()
    }

    #[allow(dead_code)]
    pub fn event_manager(&mut self) -> &mut EventManagerWithBoundedMpsc {
        self.event_man_wrapper.event_manager()
    }

    pub fn periodic_operation(&mut self) {
        self.pus_event_handler.handle_event_requests();
        self.event_man_wrapper.try_event_routing();
        self.pus_event_handler.generate_pus_event_tm();
    }
}
