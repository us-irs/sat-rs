#!/usr/bin/env python3
from socket import AF_INET, SOCK_DGRAM, socket
import msgpack

nadine = {
    "age": 24,
    "name": "Nadine",
}

msg_pack_stuff = msgpack.packb(nadine)
assert msg_pack_stuff is not None
server_socket = socket(AF_INET, SOCK_DGRAM)
target_address = "localhost", 7301
bytes_sent = server_socket.sendto(msg_pack_stuff, target_address)
recv_back = server_socket.recv(4096)
print(f"recv back: {recv_back}")

unpacked = msgpack.unpackb(recv_back)
print(f"unpacked: {unpacked}")
# human_test = {:x for x in unpacked}
loaded_back = msgpack.loads(recv_back)
print(loaded_back)
