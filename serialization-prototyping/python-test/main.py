#!/usr/bin/env python3
from socket import AF_INET, SOCK_DGRAM, socket
import msgpack


TEST_PERSON = {
    "age": 24,
    "name": "Nadine",
}


def msg_pack_unloading(recv_back: bytes):
    unpacked = msgpack.unpackb(recv_back)
    print(f"unpacked: {unpacked}")
    # human_test = {:x for x in unpacked}
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
    pass


if __name__ == "__main__":
    main()
