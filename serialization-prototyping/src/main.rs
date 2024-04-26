use rmp_serde::{Deserializer, Serializer};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::UdpSocket};

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub enum Color {
    Red = 0,
    Green = 1,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
struct Human {
    age: u16,
    name: String,

}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
struct HumanAdvanced {
    id: u32,
    age: u16,
    name: String,
    fav_color: Color,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
struct HumanGroup {
    humans: Vec<HumanAdvanced>,
    bank: HashMap<u32, usize>,
}

fn wild_testing() {
    let mut buf = Vec::new();
    let john = HumanAdvanced {
        id: 0,
        age: 42,
        name: "John".into(),
        fav_color: Color::Green,
    };

    john.serialize(&mut Serializer::new(&mut buf)).unwrap();

    println!("{:?}", buf);
    let new_val: HumanAdvanced = rmp_serde::from_slice(&buf).expect("deserialization failed");
    let rmpv_val: rmpv::Value = rmp_serde::from_slice(&buf).expect("serialization into val failed");
    println!("RMPV value: {:?}", rmpv_val);
    let json_str = serde_json::to_string(&rmpv_val).expect("creating json failed");
    assert_eq!(john, new_val);
    println!("JSON str: {}", json_str);
    let val_test: HumanAdvanced = serde_json::from_str(&json_str).expect("wild");
    println!("val test: {:?}", val_test);

    let nadine = HumanAdvanced {
        id: 1,
        age: 24,
        name: "Nadine".into(),
        fav_color: Color::Red,
    };
    let mut bank = HashMap::default();
    bank.insert(john.id, 1000000);
    bank.insert(nadine.id, 1);

    let human_group = HumanGroup {
        humans: vec![john, nadine.clone()],
        bank,
    };
    let json_str = serde_json::to_string(&nadine).unwrap();
    println!("Nadine as JSON: {}", json_str);

    let nadine_is_back: HumanAdvanced = serde_json::from_str(&json_str).unwrap();
    println!("nadine deserialized: {:?}", nadine_is_back);

    let human_group_json = serde_json::to_string(&human_group).unwrap();
    println!("human group: {}", human_group_json);
    println!("human group json size: {}", human_group_json.len());

    let human_group_rmp_vec = rmp_serde::to_vec(&human_group_json).unwrap();
    println!("human group msg pack size: {:?}", human_group_rmp_vec.len());
}

fn main() {
    let socket = UdpSocket::bind("127.0.0.1:7301").expect("binding UDP socket failed");

    // Receives a single datagram message on the socket. If `buf` is too small to hold
    // the message, it will be cut off.
    let mut buf = [0; 4096];
    let (received, src) = socket.recv_from(&mut buf).expect("receive call failed");
    let human_from_python: rmpv::Value = rmp_serde::from_slice(&buf[..received]).expect("blablah");
    let human_attempt_2: Human = rmp_serde::from_slice(&buf[..received]).expect("blhfwhfw");
    println!("human from python: {}", human_from_python);
    println!("human 2 from python: {:?}", human_attempt_2);
    let send_back_human = rmp_serde::to_vec(&human_attempt_2).expect("k32k323k2");
    socket.send_to(&send_back_human, src).expect("sending back failed");
}
