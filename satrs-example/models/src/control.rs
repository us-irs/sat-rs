use crate::Message;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
pub enum Event {
    TestEvent,
}

impl Message for Event {
    fn message_type(&self) -> crate::MessageType {
        crate::MessageType::Event
    }
}

pub mod request {
    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum Request {
        Ping,
        TestEvent,
    }
}

pub mod response {
    use crate::Message;

    #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug)]
    pub enum Response {
        Ok,
        Event(super::Event),
    }

    impl Message for Response {
        fn message_type(&self) -> crate::MessageType {
            match self {
                Response::Ok => crate::MessageType::Verification,
                Response::Event(_event) => crate::MessageType::Event,
            }
        }
    }
}
