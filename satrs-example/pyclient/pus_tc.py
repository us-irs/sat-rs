import datetime
import logging

from spacepackets.ccsds import CdsShortTimestamp
from spacepackets.ecss import PusTelecommand
from tmtccmd.config import CmdTreeNode
from tmtccmd.tmtc import DefaultPusQueueHelper
from tmtccmd.pus.s11_tc_sched import create_time_tagged_cmd
from tmtccmd.pus.tc.s3_fsfw_hk import create_request_one_hk_command

from common import (
    EXAMPLE_PUS_APID,
    make_addressable_id,
    RequestTargetId,
    AcsHkIds,
)

_LOGGER = logging.getLogger(__name__)


def create_cmd_definition_tree() -> CmdTreeNode:

    root_node = CmdTreeNode.root_node()

    test_node = CmdTreeNode("test", "Test Node")
    test_node.add_child(CmdTreeNode("ping", "Send PUS ping TC"))
    test_node.add_child(CmdTreeNode("trigger_event", "Send PUS test to trigger event"))
    root_node.add_child(test_node)

    scheduler_node = CmdTreeNode("scheduler", "Scheduler Node")
    scheduler_node.add_child(
        CmdTreeNode(
            "schedule_ping_10_secs_ahead", "Schedule Ping to execute in 10 seconds"
        )
    )
    root_node.add_child(scheduler_node)

    acs_node = CmdTreeNode("acs", "ACS Subsystem Node")
    mgm_node = CmdTreeNode("mgms", "MGM devices node")
    mgm_node.add_child(CmdTreeNode("one_shot_hk", "Request one shot HK"))
    acs_node.add_child(mgm_node)
    root_node.add_child(acs_node)

    return root_node


def pack_pus_telecommands(q: DefaultPusQueueHelper, cmd_path: str):
    cmd_path_list = cmd_path.split("/")
    if len(cmd_path_list) == 0:
        _LOGGER.warning("empty command path")
        return
    if cmd_path_list[0] == "test":
        assert len(cmd_path_list) >= 2
        if cmd_path_list[1] == "ping":
            q.add_log_cmd("Sending PUS ping telecommand")
            return q.add_pus_tc(PusTelecommand(service=17, subservice=1))
        elif cmd_path_list[1] == "trigger_event":
            q.add_log_cmd("Triggering test event")
            return q.add_pus_tc(PusTelecommand(service=17, subservice=128))
    if cmd_path_list[0] == "scheduler":
        assert len(cmd_path_list) >= 2
        if cmd_path_list[1] == "schedule_ping_10_secs_ahead":
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
    if cmd_path_list[0] == "acs":
        assert len(cmd_path_list) >= 2
        if cmd_path_list[1] == "mgm":
            assert len(cmd_path_list) >= 3
            if cmd_path_list[2] == "one_shot_hk":
                q.add_log_cmd("Sending HK one shot request")
                q.add_pus_tc(
                    create_request_one_hk_command(
                        make_addressable_id(RequestTargetId.ACS, AcsHkIds.MGM_SET)
                    )
                )
