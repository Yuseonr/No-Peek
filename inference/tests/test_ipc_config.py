"""Unit test untuk konfigurasi .env dan protokol IPC JSON Lines."""

import io
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

from no_peek.config import AppConfig, load_dotenv
from no_peek.ipc import IpcChannel, build_state_payload, format_alert_message


class ConfigAndIpcTests(unittest.TestCase):
    """Pastikan pembacaan .env dan serialisasi JSONL IPC bekerja deterministik."""

    def test_load_dotenv_and_app_config(self):
        with TemporaryDirectory() as tmp:
            env_file = Path(tmp) / ".env"
            env_file.write_text(
                "NOPEEK_ENV=prod\nNOPEEK_CAMERA_INDEX=2\nNOPEEK_IDLE_WIDTH=150.0\n",
                encoding="utf-8",
            )
            parsed = load_dotenv(env_file)
            self.assertEqual(parsed["NOPEEK_ENV"], "prod")
            self.assertEqual(parsed["NOPEEK_CAMERA_INDEX"], "2")

    def test_format_alert_and_build_state_payload_no_emoji(self):
        observations = [
            {
                "id": 1,
                "role": "OWNER",
                "state": "MONITORING",
                "position": "CENTER",
                "distance_cm": 55.0,
                "risk_s": 0.0,
            },
            {
                "id": 2,
                "role": "OBSERVER",
                "state": "ALERT",
                "position": "RIGHT",
                "distance_cm": 95.0,
                "risk_s": 2.6,
            },
        ]
        payload = build_state_payload(
            observations=observations,
            calibrated=True,
            calibration_mode="AUTO",
            auto_calibrate=True,
            owner_id=1,
            paused=False,
            fps=30.0,
            mirror=True,
        )
        self.assertEqual(payload["level"], "ALERT")
        self.assertTrue(payload["alert_active"])
        self.assertEqual(payload["icon"], "eye_warning")
        # Karena mirror=True, RIGHT dibalik menjadi LEFT ("Kiri")
        self.assertIn("Kiri", payload["message"])
        # Pastikan tidak ada karakter emoji unicode (semua ASCII)
        self.assertTrue(all(ord(ch) < 128 for ch in payload["message"]))

    def test_ipc_channel_deduplication_and_command_queue(self):
        out_buf = io.StringIO()
        in_buf = io.StringIO('{"cmd":"calibrate"}\n{"cmd":"toggle_auto_calibrate"}\n')
        channel = IpcChannel(out_stream=out_buf, in_stream=in_buf)
        channel.start_reader()

        # Tunggu sebentar hingga thread pembaca memproses in_buf
        import time
        time.sleep(0.05)

        cmd1 = channel.poll_command()
        cmd2 = channel.poll_command()
        self.assertEqual(cmd1, {"cmd": "calibrate"})
        self.assertEqual(cmd2, {"cmd": "toggle_auto_calibrate"})
        self.assertIsNone(channel.poll_command())

        sample_state = {
            "type": "state_update",
            "level": "MONITORING",
            "alert_active": False,
            "message": "",
            "calibrated": True,
            "calibration_mode": "AUTO",
            "auto_calibrate": True,
            "paused": False,
            "observer_count": 0,
        }
        self.assertTrue(channel.emit_state_if_needed(sample_state, heartbeat_s=10.0))
        # Panggilan kedua dengan state identik sebelum heartbeat tidak mengirim ulang
        self.assertFalse(channel.emit_state_if_needed(sample_state, heartbeat_s=10.0))
        lines = out_buf.getvalue().strip().splitlines()
        self.assertEqual(len(lines), 1)


if __name__ == "__main__":
    unittest.main()
