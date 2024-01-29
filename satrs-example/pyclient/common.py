from __future__ import annotations

import dataclasses
import enum
import struct

from spacepackets.ecss.tc import PacketId, PacketType

EXAMPLE_PUS_APID = 0x02
EXAMPLE_PUS_PACKET_ID_TM = PacketId(PacketType.TM, True, EXAMPLE_PUS_APID)
TM_PACKET_IDS = [EXAMPLE_PUS_PACKET_ID_TM]


class EventSeverity(enum.IntEnum):
    INFO = 0
    LOW = 1
    MEDIUM = 2
    HIGH = 3


@dataclasses.dataclass
class EventU32:
    severity: EventSeverity
    group_id: int
    unique_id: int

    @classmethod
    def unpack(cls, data: bytes) -> EventU32:
        if len(data) < 4:
            raise ValueError("passed data too short")
        event_raw = struct.unpack("!I", data[0:4])[0]
        return cls(
            severity=EventSeverity((event_raw >> 30) & 0b11),
            group_id=(event_raw >> 16) & 0x3FFF,
            unique_id=event_raw & 0xFFFF,
        )


class RequestTargetId(enum.IntEnum):
    ACS = 1


class AcsHkIds(enum.IntEnum):
    MGM_SET = 1


class HkOpCodes:
    GENERATE_ONE_SHOT = ["0", "oneshot"]


def make_addressable_id(target_id: int, unique_id: int) -> bytes:
    byte_string = bytearray(struct.pack("!I", target_id))
    byte_string.extend(struct.pack("!I", unique_id))
    return byte_string
