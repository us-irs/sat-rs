from __future__ import annotations

import dataclasses
import enum
import struct


class Apid(enum.IntEnum):
    SCHED = 1
    GENERIC_PUS = 2
    ACS = 3
    CFDP = 4
    TMTC = 5


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


class AcsId(enum.IntEnum):
    MGM_0 = 0


class AcsHkIds(enum.IntEnum):
    MGM_SET = 1


def make_addressable_id(target_id: int, unique_id: int) -> bytes:
    byte_string = bytearray(struct.pack("!I", target_id))
    byte_string.extend(struct.pack("!I", unique_id))
    return byte_string
