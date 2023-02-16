import datetime

from spacepackets.ccsds import CdsShortTimestamp
from spacepackets.ecss import PusTelecommand
from tmtccmd.config import CoreServiceList
from tmtccmd.tc import DefaultPusQueueHelper
from tmtccmd.tc.pus_11_tc_sched import create_time_tagged_cmd
from tmtccmd.tc.pus_3_fsfw_hk import create_request_one_hk_command

from common import (
    EXAMPLE_PUS_APID,
    HkOpCodes,
    make_addressable_id,
    RequestTargetId,
    AcsHkIds,
)


def pack_pus_telecommands(q: DefaultPusQueueHelper, service: str, op_code: str):
    if (
        service == CoreServiceList.SERVICE_17
        or service == CoreServiceList.SERVICE_17_ALT
    ):
        if op_code == "ping":
            q.add_log_cmd("Sending PUS ping telecommand")
            return q.add_pus_tc(PusTelecommand(service=17, subservice=1))
        elif op_code == "trigger_event":
            q.add_log_cmd("Triggering test event")
            return q.add_pus_tc(PusTelecommand(service=17, subservice=128))
    if service == CoreServiceList.SERVICE_11:
        q.add_log_cmd("Sending PUS scheduled TC telecommand")
        crt_time = CdsShortTimestamp.from_now()
        time_stamp = crt_time + datetime.timedelta(seconds=10)
        time_stamp = time_stamp.pack()
        return q.add_pus_tc(
            create_time_tagged_cmd(
                time_stamp,
                PusTelecommand(service=17, subservice=1),
                apid=EXAMPLE_PUS_APID,
            )
        )
    if service == CoreServiceList.SERVICE_3:
        if op_code in HkOpCodes.GENERATE_ONE_SHOT:
            q.add_log_cmd("Sending HK one shot request")
            q.add_pus_tc(
                create_request_one_hk_command(
                    make_addressable_id(RequestTargetId.ACS, AcsHkIds.MGM_SET)
                )
            )
        pass
