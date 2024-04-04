#!/usr/bin/env python3
"""Example client for the sat-rs example application"""
import logging
import sys
import time
from typing import Any, Optional
from prompt_toolkit.history import History
from prompt_toolkit.history import FileHistory

from spacepackets.ccsds import PacketId, PacketType
import tmtccmd
from spacepackets.ecss import PusTelemetry, PusVerificator
from spacepackets.ecss.pus_17_test import Service17Tm
from spacepackets.ecss.pus_1_verification import UnpackParams, Service1Tm
from spacepackets.ccsds.time import CdsShortTimestamp

from tmtccmd import TcHandlerBase, ProcedureParamsWrapper
from tmtccmd.core.base import BackendRequest
from tmtccmd.pus import VerificationWrapper
from tmtccmd.tmtc import CcsdsTmHandler, GenericApidHandlerBase
from tmtccmd.com import ComInterface
from tmtccmd.config import (
    CmdTreeNode,
    default_json_path,
    SetupParams,
    HookBase,
    params_to_procedure_conversion,
)
from tmtccmd.config import PreArgsParsingWrapper, SetupWrapper
from tmtccmd.logging import add_colorlog_console_logger
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
    QueueWrapper,
)
from spacepackets.seqcount import FileSeqCountProvider, PusFileSeqCountProvider
from tmtccmd.util.obj_id import ObjectIdDictT


import pus_tc
from common import Apid, EventU32

_LOGGER = logging.getLogger()


class SatRsConfigHook(HookBase):
    def __init__(self, json_cfg_path: str):
        super().__init__(json_cfg_path=json_cfg_path)

    def get_communication_interface(self, com_if_key: str) -> Optional[ComInterface]:
        from tmtccmd.config.com import (
            create_com_interface_default,
            create_com_interface_cfg_default,
        )

        assert self.cfg_path is not None
        packet_id_list = []
        for apid in Apid:
            packet_id_list.append(PacketId(PacketType.TM, True, apid))
        cfg = create_com_interface_cfg_default(
            com_if_key=com_if_key,
            json_cfg_path=self.cfg_path,
            space_packet_ids=packet_id_list,
        )
        assert cfg is not None
        return create_com_interface_default(cfg)

    def get_command_definitions(self) -> CmdTreeNode:
        """This function should return the root node of the command definition tree."""
        return pus_tc.create_cmd_definition_tree()

    def get_cmd_history(self) -> Optional[History]:
        """Optionlly return a history class for the past command paths which will be used
        when prompting a command path from the user in CLI mode."""
        return FileHistory(".tmtc-history.txt")

    def get_object_ids(self) -> ObjectIdDictT:
        from tmtccmd.config.objects import get_core_object_ids

        return get_core_object_ids()


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
            pus_tm = PusTelemetry.unpack(packet, time_reader=CdsShortTimestamp.empty())
        except ValueError as e:
            _LOGGER.warning("Could not generate PUS TM object from raw data")
            _LOGGER.warning(f"Raw Packet: [{packet.hex(sep=',')}], REPR: {packet!r}")
            raise e
        service = pus_tm.service
        if service == 1:
            tm_packet = Service1Tm.unpack(
                data=packet, params=UnpackParams(CdsShortTimestamp.empty(), 1, 2)
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
            _LOGGER.info("No handling for HK packets implemented")
            _LOGGER.info(f"Raw packet: 0x[{packet.hex(sep=',')}]")
            pus_tm = PusTelemetry.unpack(packet, time_reader=CdsShortTimestamp.empty())
            if pus_tm.subservice == 25:
                if len(pus_tm.source_data) < 8:
                    raise ValueError("No addressable ID in HK packet")
                json_str = pus_tm.source_data[8:]
                _LOGGER.info(json_str)
        elif service == 5:
            tm_packet = PusTelemetry.unpack(
                packet, time_reader=CdsShortTimestamp.empty()
            )
            src_data = tm_packet.source_data
            event_u32 = EventU32.unpack(src_data)
            _LOGGER.info(f"Received event packet. Event: {event_u32}")
            if event_u32.group_id == 0 and event_u32.unique_id == 0:
                _LOGGER.info("Received test event")
        elif service == 17:
            tm_packet = Service17Tm.unpack(
                packet, time_reader=CdsShortTimestamp.empty()
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
            tm_packet = PusTelemetry.unpack(
                packet, time_reader=CdsShortTimestamp.empty()
            )
        self.raw_logger.log_tm(pus_tm)


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
        if info.proc_type == TcProcedureType.DEFAULT:
            def_proc = info.to_def_procedure()
            _LOGGER.info(f"Queue handling finished for command {def_proc.cmd_path}")

    def feed_cb(self, info: ProcedureWrapper, wrapper: FeedWrapper):
        q = self.queue_helper
        q.queue_wrapper = wrapper.queue_wrapper
        if info.proc_type == TcProcedureType.DEFAULT:
            def_proc = info.to_def_procedure()
            assert def_proc.cmd_path is not None
            pus_tc.pack_pus_telecommands(q, def_proc.cmd_path)


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
    ccsds_handler = CcsdsTmHandler(generic_handler=tm_handler)
    # TODO: We could add the CFDP handler for the CFDP APID at a later stage.
    # ccsds_handler.add_apid_handler(tm_handler)

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
