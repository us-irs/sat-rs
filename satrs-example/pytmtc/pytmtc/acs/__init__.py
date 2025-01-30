from tmtccmd.config import CmdTreeNode


def create_acs_node(mode_node: CmdTreeNode, hk_node: CmdTreeNode) -> CmdTreeNode:
    acs_node = CmdTreeNode("acs", "ACS Subsystem Node")
    mgm_node = CmdTreeNode("mgms", "MGM devices node")
    mgm_node.add_child(mode_node)
    mgm_node.add_child(hk_node)

    acs_node.add_child(mgm_node)
    return acs_node
