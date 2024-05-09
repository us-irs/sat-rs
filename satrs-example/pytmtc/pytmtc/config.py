from typing import Optional
from prompt_toolkit.history import FileHistory, History
from spacepackets.ccsds import PacketId, PacketType
from tmtccmd import HookBase
from tmtccmd.com import ComInterface
from tmtccmd.config import CmdTreeNode
from tmtccmd.util.obj_id import ObjectIdDictT

from pytmtc.common import Apid
from pytmtc.pus_tc import create_cmd_definition_tree


class SatrsConfigHook(HookBase):
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
        return create_cmd_definition_tree()

    def get_cmd_history(self) -> Optional[History]:
        """Optionlly return a history class for the past command paths which will be used
        when prompting a command path from the user in CLI mode."""
        return FileHistory(".tmtc-history.txt")

    def get_object_ids(self) -> ObjectIdDictT:
        from tmtccmd.config.objects import get_core_object_ids

        return get_core_object_ids()
