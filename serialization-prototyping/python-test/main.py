#!/usr/bin/env python3
import enum
from socket import AF_INET, SOCK_DGRAM, socket
from pydantic import BaseModel
import msgpack


TEST_PERSON = {
    "age": 24,
    "name": "Nadine",
}


class Devices(str, enum.Enum):
    MGT = "Mgt"
    MGM = "Mgm"


class SwitchState(str, enum.Enum):
    OFF = "Off"
    ON = "On"
    UNKNOWN = "Unknown"
    FAULTY = "Faulty"


class SwitchMap(BaseModel):
    valid: bool
    switch_map: dict[Devices, SwitchState]


def msg_pack_unloading(recv_back: bytes):
    unpacked = msgpack.unpackb(recv_back)
    print(f"unpacked: {unpacked}")
    loaded_back = msgpack.loads(recv_back)
    print(loaded_back)


def main():
    server_socket = socket(AF_INET, SOCK_DGRAM)
    target_address = "localhost", 7301
    msg_pack_stuff = msgpack.packb(TEST_PERSON)
    assert msg_pack_stuff is not None
    _ = server_socket.sendto(msg_pack_stuff, target_address)
    recv_back = server_socket.recv(4096)
    print(f"recv back: {recv_back}")
    switch_map = SwitchMap.model_validate_json(recv_back)
    print(f"switch map: {switch_map}")


if __name__ == "__main__":
    main()
