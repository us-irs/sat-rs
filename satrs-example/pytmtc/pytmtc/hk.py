import struct
from spacepackets.ecss.pus_3_hk import Subservice
from spacepackets.ecss import PusService, PusTc


def create_request_one_shot_hk_cmd(apid: int, unique_id: int, set_id: int) -> PusTc:
    app_data = bytearray()
    app_data.extend(struct.pack("!I", unique_id))
    app_data.extend(struct.pack("!I", set_id))
    return PusTc(
        service=PusService.S3_HOUSEKEEPING,
        subservice=Subservice.TC_GENERATE_ONE_PARAMETER_REPORT,
        apid=apid,
        app_data=app_data,
    )
