import struct
from spacepackets.ecss import PusTc
from tmtccmd.pus.s200_fsfw_mode import Mode, Subservice
from tmtccmd.tmtc import DefaultPusQueueHelper


def create_set_mode_cmd(apid: int, unique_id: int, mode: int, submode: int) -> PusTc:
    app_data = bytearray()
    app_data.extend(struct.pack("!I", unique_id))
    app_data.extend(struct.pack("!I", mode))
    app_data.extend(struct.pack("!H", submode))
    return PusTc(
        service=200,
        subservice=Subservice.TC_MODE_COMMAND,
        apid=apid,
        app_data=app_data,
    )


def handle_set_mode_cmd(
    q: DefaultPusQueueHelper, target_str: str, mode_str: str, apid: int, unique_id: int
):
    if mode_str == "off":
        q.add_log_cmd(f"Sending Mode OFF to {target_str}")
        q.add_pus_tc(create_set_mode_cmd(apid, unique_id, Mode.OFF, 0))
    elif mode_str == "on":
        q.add_log_cmd(f"Sending Mode ON to {target_str}")
        q.add_pus_tc(create_set_mode_cmd(apid, unique_id, Mode.ON, 0))
    elif mode_str == "normal":
        q.add_log_cmd(f"Sending Mode NORMAL to {target_str}")
        q.add_pus_tc(create_set_mode_cmd(apid, unique_id, Mode.NORMAL, 0))
