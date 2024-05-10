import logging
from typing import Any
from spacepackets.ccsds.time import CdsShortTimestamp
from spacepackets.ecss import PusTm
from spacepackets.ecss.pus_17_test import Service17Tm
from spacepackets.ecss.pus_1_verification import Service1Tm, UnpackParams
from tmtccmd.logging.pus import RawTmtcTimedLogWrapper
from tmtccmd.pus import VerificationWrapper
from tmtccmd.tmtc import GenericApidHandlerBase

from pytmtc.common import Apid, EventU32
from pytmtc.hk import handle_hk_packet


_LOGGER = logging.getLogger(__name__)


class PusHandler(GenericApidHandlerBase):
    def __init__(
        self,
        file_logger: logging.Logger,
        verif_wrapper: VerificationWrapper,
        raw_logger: RawTmtcTimedLogWrapper,
    ):
        super().__init__(None)
        self.file_logger = file_logger
        self.raw_logger = raw_logger
        self.verif_wrapper = verif_wrapper

    def handle_tm(self, apid: int, packet: bytes, _user_args: Any):
        try:
            pus_tm = PusTm.unpack(
                packet, timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE
            )
        except ValueError as e:
            _LOGGER.warning("Could not generate PUS TM object from raw data")
            _LOGGER.warning(f"Raw Packet: [{packet.hex(sep=',')}], REPR: {packet!r}")
            raise e
        service = pus_tm.service
        if service == 1:
            tm_packet = Service1Tm.unpack(
                data=packet, params=UnpackParams(CdsShortTimestamp.TIMESTAMP_SIZE, 1, 2)
            )
            res = self.verif_wrapper.add_tm(tm_packet)
            if res is None:
                _LOGGER.info(
                    f"Received Verification TM[{tm_packet.service}, {tm_packet.subservice}] "
                    f"with Request ID {tm_packet.tc_req_id.as_u32():#08x}"
                )
                _LOGGER.warning(
                    f"No matching telecommand found for {tm_packet.tc_req_id}"
                )
            else:
                self.verif_wrapper.log_to_console(tm_packet, res)
                self.verif_wrapper.log_to_file(tm_packet, res)
        elif service == 3:
            pus_tm = PusTm.unpack(
                packet, timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE
            )
            handle_hk_packet(pus_tm)
        elif service == 5:
            tm_packet = PusTm.unpack(
                packet, timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE
            )
            src_data = tm_packet.source_data
            event_u32 = EventU32.unpack(src_data)
            _LOGGER.info(
                f"Received event packet. Source APID: {Apid(tm_packet.apid)!r}, Event: {event_u32}"
            )
            if event_u32.group_id == 0 and event_u32.unique_id == 0:
                _LOGGER.info("Received test event")
        elif service == 17:
            tm_packet = Service17Tm.unpack(
                packet, timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE
            )
            if tm_packet.subservice == 2:
                self.file_logger.info("Received Ping Reply TM[17,2]")
                _LOGGER.info("Received Ping Reply TM[17,2]")
            else:
                self.file_logger.info(
                    f"Received Test Packet with unknown subservice {tm_packet.subservice}"
                )
                _LOGGER.info(
                    f"Received Test Packet with unknown subservice {tm_packet.subservice}"
                )
        else:
            _LOGGER.info(
                f"The service {service} is not implemented in Telemetry Factory"
            )
            tm_packet = PusTm.unpack(
                packet, timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE
            )
        self.raw_logger.log_tm(pus_tm)
