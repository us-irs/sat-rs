from unittest import TestCase

from spacepackets.ccsds import CdsShortTimestamp
from tmtccmd.tmtc import DefaultPusQueueHelper, QueueEntryHelper
from tmtccmd.tmtc.queue import QueueWrapper

from pytmtc.config import SatrsConfigHook
from pytmtc.pus_tc import pack_pus_telecommands


class TestTcModules(TestCase):
    def setUp(self):
        self.hook = SatrsConfigHook(json_cfg_path="tmtc_conf.json")
        self.queue_helper = DefaultPusQueueHelper(
            queue_wrapper=QueueWrapper.empty(),
            tc_sched_timestamp_len=CdsShortTimestamp.TIMESTAMP_SIZE,
            seq_cnt_provider=None,
            pus_verificator=None,
            default_pus_apid=None,
        )

    def test_cmd_tree_creation_works_without_errors(self):
        cmd_defs = self.hook.get_command_definitions()
        self.assertIsNotNone(cmd_defs)

    def test_ping_cmd_generation(self):
        pack_pus_telecommands(self.queue_helper, "/test/ping")
        queue_entry = self.queue_helper.queue_wrapper.queue.popleft()
        entry_helper = QueueEntryHelper(queue_entry)
        log_queue = entry_helper.to_log_entry()
        self.assertEqual(log_queue.log_str, "Sending PUS ping telecommand")
        queue_entry = self.queue_helper.queue_wrapper.queue.popleft()
        entry_helper.entry = queue_entry
        pus_tc_entry = entry_helper.to_pus_tc_entry()
        self.assertEqual(pus_tc_entry.pus_tc.service, 17)
        self.assertEqual(pus_tc_entry.pus_tc.subservice, 1)

    def test_event_trigger_generation(self):
        pack_pus_telecommands(self.queue_helper, "/test/trigger_event")
        queue_entry = self.queue_helper.queue_wrapper.queue.popleft()
        entry_helper = QueueEntryHelper(queue_entry)
        log_queue = entry_helper.to_log_entry()
        self.assertEqual(log_queue.log_str, "Triggering test event")
        queue_entry = self.queue_helper.queue_wrapper.queue.popleft()
        entry_helper.entry = queue_entry
        pus_tc_entry = entry_helper.to_pus_tc_entry()
        self.assertEqual(pus_tc_entry.pus_tc.service, 17)
        self.assertEqual(pus_tc_entry.pus_tc.subservice, 128)
