use std::sync::mpsc::{self};

use satrs::{
    event_man::{
        EventManagerWithBoundedMpsc, EventSendProvider, EventU32SenderMpscBounded,
        MpscEventReceiver,
    },
    events::EventU32,
    params::Params,
    pus::{
        event_man::{
            DefaultPusEventU32Dispatcher, EventReporter, EventRequest, EventRequestWithToken,
        },
        verification::{TcStateStarted, VerificationReportingProvider, VerificationToken},
        EcssTmSender,
    },
    spacepackets::time::cds::{self, CdsTime},
};
use satrs_example::config::PUS_APID;

use crate::update_time;

pub struct PusEventHandler<VerificationReporter: VerificationReportingProvider> {
    event_request_rx: mpsc::Receiver<EventRequestWithToken>,
    pus_event_dispatcher: DefaultPusEventU32Dispatcher<()>,
    pus_event_man_rx: mpsc::Receiver<(EventU32, Option<Params>)>,
    tm_sender: Box<dyn EcssTmSender>,
    time_provider: CdsTime,
    timestamp: [u8; 7],
    verif_handler: VerificationReporter,
}
/*
*/

impl<VerificationReporter: VerificationReportingProvider> PusEventHandler<VerificationReporter> {
    pub fn new(
        verif_handler: VerificationReporter,
        event_manager: &mut EventManagerWithBoundedMpsc,
        event_request_rx: mpsc::Receiver<EventRequestWithToken>,
        tm_sender: impl EcssTmSender,
    ) -> Self {
        let event_queue_cap = 30;
        let (pus_event_man_tx, pus_event_man_rx) = mpsc::sync_channel(event_queue_cap);

        // All events sent to the manager are routed to the PUS event manager, which generates PUS event
        // telemetry for each event.
        let event_reporter = EventReporter::new(PUS_APID, 128).unwrap();
        let pus_event_dispatcher =
            DefaultPusEventU32Dispatcher::new_with_default_backend(event_reporter);
        let pus_event_man_send_provider =
            EventU32SenderMpscBounded::new(1, pus_event_man_tx, event_queue_cap);

        event_manager.subscribe_all(pus_event_man_send_provider.channel_id());
        event_manager.add_sender(pus_event_man_send_provider);

        Self {
            event_request_rx,
            pus_event_dispatcher,
            pus_event_man_rx,
            time_provider: cds::CdsTime::new_with_u16_days(0, 0),
            timestamp: [0; 7],
            verif_handler,
            tm_sender: Box::new(tm_sender),
        }
    }

    pub fn handle_event_requests(&mut self) {
        let report_completion = |event_req: EventRequestWithToken, timestamp: &[u8]| {
            let started_token: VerificationToken<TcStateStarted> = event_req
                .token
                .try_into()
                .expect("expected start verification token");
            self.verif_handler
                .completion_success(started_token, timestamp)
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
        if let Ok((event, _param)) = self.pus_event_man_rx.try_recv() {
            update_time(&mut self.time_provider, &mut self.timestamp);
            self.pus_event_dispatcher
                .generate_pus_event_tm_generic(
                    self.tm_sender.upcast_mut(),
                    &self.timestamp,
                    event,
                    None,
                )
                .expect("Sending TM as event failed");
        }
    }
}

pub struct EventManagerWrapper {
    event_manager: EventManagerWithBoundedMpsc,
    event_sender: mpsc::Sender<(EventU32, Option<Params>)>,
}

impl EventManagerWrapper {
    pub fn new() -> Self {
        // The sender handle is the primary sender handle for all components which want to create events.
        // The event manager will receive the RX handle to receive all the events.
        let (event_sender, event_man_rx) = mpsc::channel();
        let event_recv = MpscEventReceiver::<EventU32>::new(event_man_rx);
        Self {
            event_manager: EventManagerWithBoundedMpsc::new(event_recv),
            event_sender,
        }
    }

    pub fn clone_event_sender(&self) -> mpsc::Sender<(EventU32, Option<Params>)> {
        self.event_sender.clone()
    }

    pub fn event_manager(&mut self) -> &mut EventManagerWithBoundedMpsc {
        &mut self.event_manager
    }

    pub fn try_event_routing(&mut self) {
        // Perform the event routing.
        self.event_manager
            .try_event_handling()
            .expect("event handling failed");
    }
}

pub struct EventHandler<VerificationReporter: VerificationReportingProvider> {
    pub event_man_wrapper: EventManagerWrapper,
    pub pus_event_handler: PusEventHandler<VerificationReporter>,
}

impl<VerificationReporter: VerificationReportingProvider> EventHandler<VerificationReporter> {
    pub fn new(
        tm_sender: impl EcssTmSender,
        verif_handler: VerificationReporter,
        event_request_rx: mpsc::Receiver<EventRequestWithToken>,
    ) -> Self {
        let mut event_man_wrapper = EventManagerWrapper::new();
        let pus_event_handler = PusEventHandler::new(
            verif_handler,
            event_man_wrapper.event_manager(),
            event_request_rx,
            tm_sender,
        );
        Self {
            event_man_wrapper,
            pus_event_handler,
        }
    }

    pub fn clone_event_sender(&self) -> mpsc::Sender<(EventU32, Option<Params>)> {
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
