from tmtccmd.config import OpCodeEntry, TmtcDefinitionWrapper, CoreServiceList
from tmtccmd.config.globals import get_default_tmtc_defs

from common import HkOpCodes


def tc_definitions() -> TmtcDefinitionWrapper:
    defs = get_default_tmtc_defs()
    srv_5 = OpCodeEntry()
    srv_5.add("0", "Event Test")
    defs.add_service(
        name=CoreServiceList.SERVICE_5.value,
        info="PUS Service 5 Event",
        op_code_entry=srv_5,
    )
    srv_17 = OpCodeEntry()
    srv_17.add("ping", "Ping Test")
    srv_17.add("trigger_event", "Trigger Event")
    defs.add_service(
        name=CoreServiceList.SERVICE_17_ALT,
        info="PUS Service 17 Test",
        op_code_entry=srv_17,
    )
    srv_3 = OpCodeEntry()
    srv_3.add(HkOpCodes.GENERATE_ONE_SHOT, "Generate AOCS one shot HK")
    defs.add_service(
        name=CoreServiceList.SERVICE_3,
        info="PUS Service 3 Housekeeping",
        op_code_entry=srv_3,
    )
    srv_11 = OpCodeEntry()
    srv_11.add("0", "Scheduled TC Test")
    defs.add_service(
        name=CoreServiceList.SERVICE_11,
        info="PUS Service 11 TC Scheduling",
        op_code_entry=srv_11,
    )
    return defs
