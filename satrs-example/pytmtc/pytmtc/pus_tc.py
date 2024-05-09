import datetime
import struct
import logging

from spacepackets.ccsds import CdsShortTimestamp
from spacepackets.ecss import PusTelecommand
from spacepackets.seqcount import FileSeqCountProvider
from tmtccmd import ProcedureWrapper, TcHandlerBase
from tmtccmd.config import CmdTreeNode
from tmtccmd.pus import VerificationWrapper
from tmtccmd.pus.tc.s200_fsfw_mode import Mode
from tmtccmd.tmtc import (
    DefaultPusQueueHelper,
    FeedWrapper,
    QueueWrapper,
    SendCbParams,
    TcProcedureType,
    TcQueueEntryType,
)
from tmtccmd.pus.s11_tc_sched import create_time_tagged_cmd
from tmtccmd.pus.s200_fsfw_mode import Subservice as ModeSubservice

from pytmtc.common import AcsId, Apid

_LOGGER = logging.getLogger(__name__)


class TcHandler(TcHandlerBase):
    def __init__(
        self,
        seq_count_provider: FileSeqCountProvider,
        verif_wrapper: VerificationWrapper,
    ):
        super(TcHandler, self).__init__()
        self.seq_count_provider = seq_count_provider
        self.verif_wrapper = verif_wrapper
        self.queue_helper = DefaultPusQueueHelper(
            queue_wrapper=QueueWrapper.empty(),
            tc_sched_timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE,
            seq_cnt_provider=seq_count_provider,
            pus_verificator=self.verif_wrapper.pus_verificator,
            default_pus_apid=None,
        )

    def send_cb(self, send_params: SendCbParams):
        entry_helper = send_params.entry
        if entry_helper.is_tc:
            if entry_helper.entry_type == TcQueueEntryType.PUS_TC:
                pus_tc_wrapper = entry_helper.to_pus_tc_entry()
                raw_tc = pus_tc_wrapper.pus_tc.pack()
                _LOGGER.info(f"Sending {pus_tc_wrapper.pus_tc}")
                send_params.com_if.send(raw_tc)
        elif entry_helper.entry_type == TcQueueEntryType.LOG:
            log_entry = entry_helper.to_log_entry()
            _LOGGER.info(log_entry.log_str)

    def queue_finished_cb(self, info: ProcedureWrapper):
        if info.proc_type == TcProcedureType.TREE_COMMANDING:
            def_proc = info.to_tree_commanding_procedure()
            _LOGGER.info(f"Queue handling finished for command {def_proc.cmd_path}")

    def feed_cb(self, info: ProcedureWrapper, wrapper: FeedWrapper):
        q = self.queue_helper
        q.queue_wrapper = wrapper.queue_wrapper
        if info.proc_type == TcProcedureType.TREE_COMMANDING:
            def_proc = info.to_tree_commanding_procedure()
            assert def_proc.cmd_path is not None
            pack_pus_telecommands(q, def_proc.cmd_path)


def create_cmd_definition_tree() -> CmdTreeNode:

    root_node = CmdTreeNode.root_node()

    hk_node = CmdTreeNode("hk", "Housekeeping Node", hide_children_for_print=True)
    hk_node.add_child(CmdTreeNode("one_shot_hk", "Request One Shot HK set"))
    hk_node.add_child(
        CmdTreeNode("enable", "Enable periodic housekeeping data generation")
    )
    hk_node.add_child(
        CmdTreeNode("disable", "Disable periodic housekeeping data generation")
    )

    mode_node = CmdTreeNode("mode", "Mode Node", hide_children_for_print=True)
    set_mode_node = CmdTreeNode(
        "set_mode", "Set Node", hide_children_which_are_leaves=True
    )
    set_mode_node.add_child(CmdTreeNode("off", "Set OFF Mode"))
    set_mode_node.add_child(CmdTreeNode("on", "Set ON Mode"))
    set_mode_node.add_child(CmdTreeNode("normal", "Set NORMAL Mode"))
    mode_node.add_child(set_mode_node)
    mode_node.add_child(CmdTreeNode("read_mode", "Read Mode"))

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
    mgm_node.add_child(mode_node)
    mgm_node.add_child(hk_node)

    acs_node.add_child(mgm_node)
    root_node.add_child(acs_node)

    return root_node


def create_set_mode_cmd(
    apid: int, unique_id: int, mode: int, submode: int
) -> PusTelecommand:
    app_data = bytearray()
    app_data.extend(struct.pack("!I", unique_id))
    app_data.extend(struct.pack("!I", mode))
    app_data.extend(struct.pack("!H", submode))
    return PusTelecommand(
        service=200,
        subservice=ModeSubservice.TC_MODE_COMMAND,
        apid=apid,
        app_data=app_data,
    )


def pack_pus_telecommands(q: DefaultPusQueueHelper, cmd_path: str):
    # It should always be at least the root path "/", so we split of the empty portion left of it.
    cmd_path_list = cmd_path.split("/")[1:]
    if len(cmd_path_list) == 0:
        _LOGGER.warning("empty command path")
        return
    if cmd_path_list[0] == "test":
        assert len(cmd_path_list) >= 2
        if cmd_path_list[1] == "ping":
            q.add_log_cmd("Sending PUS ping telecommand")
            return q.add_pus_tc(
                PusTelecommand(apid=Apid.GENERIC_PUS, service=17, subservice=1)
            )
        elif cmd_path_list[1] == "trigger_event":
            q.add_log_cmd("Triggering test event")
            return q.add_pus_tc(
                PusTelecommand(apid=Apid.GENERIC_PUS, service=17, subservice=128)
            )
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
                    apid=Apid.SCHED,
                )
            )
    if cmd_path_list[0] == "acs":
        assert len(cmd_path_list) >= 2
        if cmd_path_list[1] == "mgms":
            assert len(cmd_path_list) >= 3
            if cmd_path_list[2] == "hk":
                if cmd_path_list[3] == "one_shot_hk":
                    q.add_log_cmd("Sending HK one shot request")
                    # TODO: Fix
                    # q.add_pus_tc(
                    #   create_request_one_hk_command(
                    #    make_addressable_id(Apid.ACS, AcsId.MGM_SET)
                    # )
                    # )
            if cmd_path_list[2] == "mode":
                if cmd_path_list[3] == "set_mode":
                    handle_set_mode_cmd(
                        q, "MGM 0", cmd_path_list[4], Apid.ACS, AcsId.MGM_0
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
