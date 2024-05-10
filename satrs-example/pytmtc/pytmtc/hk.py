import logging
import struct
from spacepackets.ecss.pus_3_hk import Subservice
from spacepackets.ecss import PusTm

from pytmtc.common import AcsId, Apid
from pytmtc.mgms import handle_mgm_hk_report


_LOGGER = logging.getLogger(__name__)


def handle_hk_packet(pus_tm: PusTm):
    if len(pus_tm.source_data) < 4:
        raise ValueError("no unique ID in HK packet")
    unique_id = struct.unpack("!I", pus_tm.source_data[:4])[0]
    if (
        pus_tm.subservice == Subservice.TM_HK_REPORT
        or pus_tm.subservice == Subservice.TM_DIAGNOSTICS_REPORT
    ):
        if len(pus_tm.source_data) < 8:
            raise ValueError("no set ID in HK packet")
        set_id = struct.unpack("!I", pus_tm.source_data[4:8])[0]
        handle_hk_report(pus_tm, unique_id, set_id)
    _LOGGER.warning(
        f"handling for HK packet with subservice {pus_tm.subservice} not implemented yet"
    )


def handle_hk_report(pus_tm: PusTm, unique_id: int, set_id: int):
    hk_data = pus_tm.source_data[8:]
    if pus_tm.apid == Apid.ACS:
        if unique_id == AcsId.MGM_0:
            handle_mgm_hk_report(pus_tm, set_id, hk_data)
        else:
            _LOGGER.warning(
                f"handling for HK report with unique ID {unique_id} not implemented yet"
            )
    else:
        _LOGGER.warning(
            f"handling for HK report with apid {pus_tm.apid} not implemented yet"
        )
