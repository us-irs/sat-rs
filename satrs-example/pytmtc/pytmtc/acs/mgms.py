import logging
import struct
import enum
from typing import List
from spacepackets.ecss import PusTm
from tmtccmd.tmtc import DefaultPusQueueHelper

from pytmtc.common import AcsId, Apid
from pytmtc.hk_common import create_request_one_shot_hk_cmd
from pytmtc.mode import handle_set_mode_cmd


_LOGGER = logging.getLogger(__name__)


class SetId(enum.IntEnum):
    SENSOR_SET = 0


def create_mgm_cmds(q: DefaultPusQueueHelper, cmd_path: List[str]):
    assert len(cmd_path) >= 3
    if cmd_path[2] == "hk":
        if cmd_path[3] == "one_shot_hk":
            q.add_log_cmd("Sending HK one shot request")
            q.add_pus_tc(
                create_request_one_shot_hk_cmd(Apid.ACS, AcsId.MGM_0, SetId.SENSOR_SET)
            )

    if cmd_path[2] == "mode":
        if cmd_path[3] == "set_mode":
            handle_set_mode_cmd(q, "MGM 0", cmd_path[4], Apid.ACS, AcsId.MGM_0)


def handle_mgm_hk_report(pus_tm: PusTm, set_id: int, hk_data: bytes):
    if set_id == SetId.SENSOR_SET:
        if len(hk_data) != 13:
            raise ValueError(f"invalid HK data length, expected 13, got {len(hk_data)}")
        data_valid = hk_data[0]
        mgm_x = struct.unpack("!f", hk_data[1:5])[0]
        mgm_y = struct.unpack("!f", hk_data[5:9])[0]
        mgm_z = struct.unpack("!f", hk_data[9:13])[0]
        _LOGGER.info(
            f"received MGM HK set in uT: Valid {data_valid} X {mgm_x} Y {mgm_y} Z {mgm_z}"
        )
        pass
