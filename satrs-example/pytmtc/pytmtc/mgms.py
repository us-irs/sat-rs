import enum
from typing import List
from tmtccmd.tmtc import DefaultPusQueueHelper

from pytmtc.common import AcsId, Apid
from pytmtc.hk import create_request_one_shot_hk_cmd
from pytmtc.mode import handle_set_mode_cmd


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
