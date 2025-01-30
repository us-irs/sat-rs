#!/usr/bin/env python3
"""Example client for the sat-rs example application"""

import logging
import sys
import time

import tmtccmd
from spacepackets.ecss import PusVerificator

from tmtccmd import ProcedureParamsWrapper
from tmtccmd.core.base import BackendRequest
from tmtccmd.pus import VerificationWrapper
from tmtccmd.tmtc import CcsdsTmHandler
from tmtccmd.config import (
    default_json_path,
    SetupParams,
    params_to_procedure_conversion,
)
from tmtccmd.config import PreArgsParsingWrapper, SetupWrapper
from tmtccmd.logging import add_colorlog_console_logger
from tmtccmd.logging.pus import (
    RegularTmtcLogWrapper,
    RawTmtcTimedLogWrapper,
    TimedLogWhen,
)
from spacepackets.seqcount import PusFileSeqCountProvider


from pytmtc.config import SatrsConfigHook
from pytmtc.pus_tc import TcHandler
from pytmtc.pus_tm import PusHandler

_LOGGER = logging.getLogger()


def main():
    add_colorlog_console_logger(_LOGGER)
    tmtccmd.init_printout(False)
    hook_obj = SatrsConfigHook(json_cfg_path=default_json_path())
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
                tmtc_backend.close_com_if()
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
        tmtc_backend.close_com_if()
        sys.exit(0)


if __name__ == "__main__":
    main()
