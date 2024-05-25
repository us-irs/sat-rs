#!/usr/bin/env python3
"""Example client for the sat-rs example application"""
import struct
import logging
import sys
import time
from typing import Any, Optional, cast
from prompt_toolkit.history import FileHistory, History
from spacepackets.ecss.tm import CdsShortTimestamp

import tmtccmd
from spacepackets.ecss import PusTelemetry, PusTelecommand, PusTm, PusVerificator
from spacepackets.ecss.pus_17_test import Service17Tm
from spacepackets.ecss.pus_1_verification import UnpackParams, Service1Tm

from tmtccmd import TcHandlerBase, ProcedureParamsWrapper
from tmtccmd.core.base import BackendRequest
from tmtccmd.core.ccsds_backend import QueueWrapper
from tmtccmd.logging import add_colorlog_console_logger
from tmtccmd.pus import VerificationWrapper
from tmtccmd.tmtc import CcsdsTmHandler, SpecificApidHandlerBase
from tmtccmd.com import ComInterface
from tmtccmd.config import (
    CmdTreeNode,
    default_json_path,
    SetupParams,
    HookBase,
    params_to_procedure_conversion,
)
from tmtccmd.config.com import SerialCfgWrapper
from tmtccmd.config import PreArgsParsingWrapper, SetupWrapper
from tmtccmd.logging.pus import (
    RegularTmtcLogWrapper,
    RawTmtcTimedLogWrapper,
    TimedLogWhen,
)
from tmtccmd.tmtc import (
    TcQueueEntryType,
    ProcedureWrapper,
    TcProcedureType,
    FeedWrapper,
    SendCbParams,
    DefaultPusQueueHelper,
)
from tmtccmd.pus.s5_fsfw_event import Service5Tm
from spacepackets.seqcount import FileSeqCountProvider, PusFileSeqCountProvider
from tmtccmd.util.obj_id import ObjectIdDictT

_LOGGER = logging.getLogger()

EXAMPLE_PUS_APID = 0x02


class SatRsConfigHook(HookBase):
    def __init__(self, json_cfg_path: str):
        super().__init__(json_cfg_path)

    def get_communication_interface(self, com_if_key: str) -> Optional[ComInterface]:
        from tmtccmd.config.com import (
            create_com_interface_default,
            create_com_interface_cfg_default,
        )

        assert self.cfg_path is not None
        cfg = create_com_interface_cfg_default(
            com_if_key=com_if_key,
            json_cfg_path=self.cfg_path,
            space_packet_ids=None,
        )
        if cfg is None:
            raise ValueError(
                f"No valid configuration could be retrieved for the COM IF with key {com_if_key}"
            )
        if cfg.com_if_key == "serial_cobs":
            cfg = cast(SerialCfgWrapper, cfg)
            cfg.serial_cfg.serial_timeout = 0.5
        return create_com_interface_default(cfg)

    def get_command_definitions(self) -> CmdTreeNode:
        """This function should return the root node of the command definition tree."""
        return create_cmd_definition_tree()

    def get_cmd_history(self) -> Optional[History]:
        """Optionlly return a history class for the past command paths which will be used
        when prompting a command path from the user in CLI mode."""
        return FileHistory(".tmtc-history.txt")

    def get_object_ids(self) -> ObjectIdDictT:
        from tmtccmd.config.objects import get_core_object_ids

        return get_core_object_ids()


def create_cmd_definition_tree() -> CmdTreeNode:
    root_node = CmdTreeNode.root_node()
    root_node.add_child(CmdTreeNode("ping", "Send PUS ping TC"))
    root_node.add_child(CmdTreeNode("change_blink_freq", "Change blink frequency"))
    return root_node


class PusHandler(SpecificApidHandlerBase):
    def __init__(
        self,
        file_logger: logging.Logger,
        verif_wrapper: VerificationWrapper,
        raw_logger: RawTmtcTimedLogWrapper,
    ):
        super().__init__(EXAMPLE_PUS_APID, None)
        self.file_logger = file_logger
        self.raw_logger = raw_logger
        self.verif_wrapper = verif_wrapper

    def handle_tm(self, packet: bytes, _user_args: Any):
        try:
            pus_tm = PusTm.unpack(
                packet, timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE
            )
        except ValueError as e:
            _LOGGER.warning("Could not generate PUS TM object from raw data")
            _LOGGER.warning(f"Raw Packet: [{packet.hex(sep=',')}], REPR: {packet!r}")
            raise e
        service = pus_tm.service
        tm_packet = None
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
        if service == 3:
            _LOGGER.info("No handling for HK packets implemented")
            _LOGGER.info(f"Raw packet: 0x[{packet.hex(sep=',')}]")
            pus_tm = PusTelemetry.unpack(packet, CdsShortTimestamp.TIMESTAMP_SIZE)
            if pus_tm.subservice == 25:
                if len(pus_tm.source_data) < 8:
                    raise ValueError("No addressable ID in HK packet")
                json_str = pus_tm.source_data[8:]
                _LOGGER.info("received JSON string: " + json_str.decode("utf-8"))
        if service == 5:
            tm_packet = Service5Tm.unpack(packet, CdsShortTimestamp.TIMESTAMP_SIZE)
        if service == 17:
            tm_packet = Service17Tm.unpack(packet, CdsShortTimestamp.TIMESTAMP_SIZE)
            if tm_packet.subservice == 2:
                _LOGGER.info("Received Ping Reply TM[17,2]")
            else:
                _LOGGER.info(
                    f"Received Test Packet with unknown subservice {tm_packet.subservice}"
                )
        if tm_packet is None:
            _LOGGER.info(
                f"The service {service} is not implemented in Telemetry Factory"
            )
            tm_packet = PusTelemetry.unpack(packet, CdsShortTimestamp.TIMESTAMP_SIZE)
        self.raw_logger.log_tm(pus_tm)


def make_addressable_id(target_id: int, unique_id: int) -> bytes:
    byte_string = bytearray(struct.pack("!I", target_id))
    byte_string.extend(struct.pack("!I", unique_id))
    return byte_string


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
            tc_sched_timestamp_len=7,
            seq_cnt_provider=seq_count_provider,
            pus_verificator=verif_wrapper.pus_verificator,
            default_pus_apid=EXAMPLE_PUS_APID,
        )

    def send_cb(self, send_params: SendCbParams):
        entry_helper = send_params.entry
        if entry_helper.is_tc:
            if entry_helper.entry_type == TcQueueEntryType.PUS_TC:
                pus_tc_wrapper = entry_helper.to_pus_tc_entry()
                pus_tc_wrapper.pus_tc.seq_count = (
                    self.seq_count_provider.get_and_increment()
                )
                self.verif_wrapper.add_tc(pus_tc_wrapper.pus_tc)
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
            cmd_path = def_proc.cmd_path
            if cmd_path == "/ping":
                q.add_log_cmd("Sending PUS ping telecommand")
                q.add_pus_tc(PusTelecommand(service=17, subservice=1))
            if cmd_path == "/change_blink_freq":
                self.create_change_blink_freq_command(q)

    def create_change_blink_freq_command(self, q: DefaultPusQueueHelper):
        q.add_log_cmd("Changing blink frequency")
        while True:
            blink_freq = int(
                input(
                    "Please specify new blink frequency in ms. Valid Range [2..10000]: "
                )
            )
            if blink_freq < 2 or blink_freq > 10000:
                print(
                    "Invalid blink frequency. Please specify a value between 2 and 10000."
                )
                continue
            break
        app_data = struct.pack("!I", blink_freq)
        q.add_pus_tc(PusTelecommand(service=8, subservice=1, app_data=app_data))


def main():
    add_colorlog_console_logger(_LOGGER)
    tmtccmd.init_printout(False)
    hook_obj = SatRsConfigHook(json_cfg_path=default_json_path())
    parser_wrapper = PreArgsParsingWrapper()
    parser_wrapper.create_default_parent_parser()
    parser_wrapper.create_default_parser()
    parser_wrapper.add_def_proc_args()
    params = SetupParams()
    post_args_wrapper = parser_wrapper.parse(hook_obj, params)
    proc_wrapper = ProcedureParamsWrapper()
    if post_args_wrapper.use_gui:
        post_args_wrapper.set_params_without_prompts(proc_wrapper)
    else:
        post_args_wrapper.set_params_with_prompts(proc_wrapper)
    params.apid = EXAMPLE_PUS_APID
    setup_args = SetupWrapper(
        hook_obj=hook_obj, setup_params=params, proc_param_wrapper=proc_wrapper
    )
    # Create console logger helper and file loggers
    tmtc_logger = RegularTmtcLogWrapper()
    file_logger = tmtc_logger.logger
    raw_logger = RawTmtcTimedLogWrapper(when=TimedLogWhen.PER_HOUR, interval=1)
    verificator = PusVerificator()
    verification_wrapper = VerificationWrapper(verificator, _LOGGER, file_logger)
    # Create primary TM handler and add it to the CCSDS Packet Handler
    tm_handler = PusHandler(file_logger, verification_wrapper, raw_logger)
    ccsds_handler = CcsdsTmHandler(generic_handler=None)
    ccsds_handler.add_apid_handler(tm_handler)

    # Create TC handler
    seq_count_provider = PusFileSeqCountProvider()
    tc_handler = TcHandler(seq_count_provider, verification_wrapper)
    tmtccmd.setup(setup_args=setup_args)
    init_proc = params_to_procedure_conversion(setup_args.proc_param_wrapper)
    tmtc_backend = tmtccmd.create_default_tmtc_backend(
        setup_wrapper=setup_args,
        tm_handler=ccsds_handler,
        tc_handler=tc_handler,
        init_procedure=init_proc,
    )
    tmtccmd.start(tmtc_backend=tmtc_backend, hook_obj=hook_obj)
    try:
        while True:
            state = tmtc_backend.periodic_op(None)
            if state.request == BackendRequest.TERMINATION_NO_ERROR:
                sys.exit(0)
            elif state.request == BackendRequest.DELAY_IDLE:
                _LOGGER.info("TMTC Client in IDLE mode")
                time.sleep(3.0)
            elif state.request == BackendRequest.DELAY_LISTENER:
                time.sleep(0.8)
            elif state.request == BackendRequest.DELAY_CUSTOM:
                if state.next_delay.total_seconds() <= 0.4:
                    time.sleep(state.next_delay.total_seconds())
                else:
                    time.sleep(0.4)
            elif state.request == BackendRequest.CALL_NEXT:
                pass
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
