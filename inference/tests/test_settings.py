from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch
from io import StringIO
import argparse
import json
import os
import queue
import subprocess
import sys
import threading
import time

import cv2
import numpy as np

from no_peek.analysis import Analyzer
from no_peek.config import save_preferences
from no_peek.preview import render_compact
from no_peek.settings import Settings
from no_peek.config import find_repo_root
from no_peek.ipc import IpcChannel
from no_peek.main import run, arguments
from no_peek.native_preview import NativePreview
from test_experiment import sample_face


class SettingsTests(unittest.TestCase):
    def test_native_preview_pixels_mirror_rate_limit_and_size(self):
        frame = np.zeros((720, 1280, 3), dtype=np.uint8)
        frame[:, :640] = (255, 0, 0)
        preview = NativePreview()
        first = preview.frame(frame, False, 10.)
        self.assertEqual((first['width'], first['height']), (320, 180))
        pixels = np.frombuffer(bytes.fromhex(first['pixels_hex']), dtype=np.uint8).reshape(180, 320, 4)
        np.testing.assert_array_equal(pixels[0, 0], (255, 0, 0, 255))
        self.assertIsNone(preview.frame(frame, True, 10.1))
        mirrored = preview.frame(frame, True, 10.3)
        mirrored_pixels = np.frombuffer(bytes.fromhex(mirrored['pixels_hex']), dtype=np.uint8).reshape(180, 320, 4)
        np.testing.assert_array_equal(mirrored_pixels, pixels[:, ::-1])
        self.assertEqual(frame.shape, (720, 1280, 3))
    def test_desktop_overrides_saved_preview_preference(self):
        with patch.dict(os.environ, {"NOPEEK_PREVIEW": "true"}), patch('sys.argv', ['no-peek', '--desktop']):
            self.assertTrue(arguments().headless)

    def test_desktop_worker_pause_resume_and_preview_rejection_over_real_pipes(self):
        with TemporaryDirectory() as directory:
            video = Path(directory) / 'desktop.avi'
            writer = cv2.VideoWriter(str(video), cv2.VideoWriter_fourcc(*'MJPG'), 30, (320, 240))
            self.assertTrue(writer.isOpened())
            for _ in range(300):
                writer.write(np.zeros((240, 320, 3), np.uint8))
            writer.release()
            events = queue.Queue()
            with (Path(directory) / 'stderr.log').open('w+') as errors:
                process = subprocess.Popen([sys.executable, '-m', 'no_peek.main', '--desktop',
                    '--ipc', 'stdio', '--video', str(video)], stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=errors, text=True,
                    env={**os.environ, 'NOPEEK_PREVIEW': 'true', 'NOPEEK_ENV': 'dev'})

                def read_events():
                    for line in process.stdout:
                        events.put(json.loads(line))
                    events.put(None)

                reader = threading.Thread(target=read_events, daemon=True)
                reader.start()

                def wait_for(predicate):
                    while True:
                        event = events.get(timeout=30)
                        self.assertIsNotNone(event, 'Worker stopped before acknowledging command')
                        if predicate(event):
                            return event

                def send(command, **values):
                    process.stdin.write(json.dumps({'cmd': command, **values}) + '\n')
                    process.stdin.flush()

                try:
                    ready = wait_for(lambda event: event['type'] == 'ready')
                    self.assertTrue(ready['headless'])
                    settings = wait_for(lambda event: event['type'] == 'settings')
                    self.assertFalse(settings['preview'])
                    send('pause')
                    wait_for(lambda event: event.get('paused') is True)
                    send('toggle_preview')
                    error = wait_for(lambda event: event['type'] == 'error')
                    self.assertFalse(error['fatal'])
                    self.assertIn('dinonaktifkan', error['message'])
                    send('resume')
                    wait_for(lambda event: event.get('paused') is False)
                    send('set_preview', enabled=True)
                    wait_for(lambda event: event['type'] == 'settings' and event['preview'])
                    preview = wait_for(lambda event: event['type'] == 'preview_frame')
                    self.assertEqual(len(bytes.fromhex(preview['pixels_hex'])), preview['width'] * preview['height'] * 4)
                    self.assertLessEqual(preview['width'], 320)
                    self.assertLessEqual(preview['height'], 240)
                    send('set_preview', enabled=False)
                    wait_for(lambda event: event['type'] == 'settings' and not event['preview'])
                    until = time.monotonic() + .35
                    while time.monotonic() < until:
                        try:
                            event = events.get(timeout=until - time.monotonic())
                        except queue.Empty:
                            break
                        self.assertIsNotNone(event)
                        self.assertNotEqual(event['type'], 'preview_frame')
                    send('set_preview', enabled=True)
                    wait_for(lambda event: event['type'] == 'preview_frame')
                    send('pause')
                    wait_for(lambda event: event['type'] == 'settings' and not event['preview'])
                    wait_for(lambda event: event.get('paused') is True)
                    send('quit')
                    self.assertEqual(process.wait(timeout=10), 0)
                finally:
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=10)
                    process.stdin.close()
                    process.stdout.close()
                    reader.join(timeout=2)

    def test_menu_changes_and_invalid_values(self):
        original = Settings(.5, 2, 7, False, True)
        changed = original.changed({"cmd": "set_alert_delay", "seconds": 5})
        self.assertEqual((changed.alert_after_s, changed.escalated_after_s), (5, 10))
        self.assertEqual(original.alert_after_s, 2)
        self.assertEqual(original.changed({"cmd": "set_confidence", "percent": 35}).confidence, .35)
        self.assertTrue(original.changed({"cmd": "toggle_preview"}).preview)
        self.assertFalse(original.changed({"cmd": "toggle_mirror"}).mirror)
        for command in ({"cmd": "set_alert_delay", "seconds": True}, {"cmd": "set_confidence", "percent": 0}, {"cmd": "unknown"}):
            with self.assertRaises(ValueError):
                original.changed(command)
        for args in ((float('nan'), 2, 7), (.5, 7, 2), (.5, 0, 7)):
            with self.assertRaises(ValueError):
                Settings(*args, False, True)

    def test_persist_preserves_other_keys_and_removes_duplicates(self):
        with TemporaryDirectory() as directory:
            target = Path(directory) / '.env'
            target.write_text('# camera\nNOPEEK_CAMERA_INDEX=2\nNOPEEK_CONFIDENCE=.2\nNOPEEK_CONFIDENCE=.1\n')
            settings = Settings(.7, 3, 8, True, False)
            save_preferences(settings.environment(), target)
            result = target.read_text()
            self.assertIn('# camera\nNOPEEK_CAMERA_INDEX=2\n', result)
            self.assertEqual(result.count('NOPEEK_CONFIDENCE='), 1)
            self.assertIn('NOPEEK_CONFIDENCE=0.7', result)
            self.assertIn('NOPEEK_PREVIEW=true', result)
            self.assertEqual(list(Path(directory).iterdir()), [target])

    def test_custom_delay_controls_real_analyzer(self):
        analyzer = Analyzer(alert_after_s=3, escalated_after_s=8)
        owner = sample_face()
        rows = analyzer.update([owner], 0, (720, 1280, 3))
        analyzer.calibrate([rows[0]['id']])
        observer = sample_face(400)
        for tick in range(81):
            rows = analyzer.update([owner, observer], tick / 10, (720, 1280, 3))
            if tick == 20:
                self.assertEqual(rows[1]['state'], 'PENDING')
            if tick == 30:
                self.assertEqual(rows[1]['state'], 'ALERT')
        self.assertEqual(rows[1]['state'], 'ESCALATED')

    def test_compact_preview_keeps_source_unchanged_and_mirrors(self):
        frame = np.zeros((240, 320, 3), np.uint8)
        frame[:, :160] = (255, 0, 0)
        original = frame.copy()
        normal = render_compact(frame, [], {"paused": False, "owner": None}, False)
        mirrored = render_compact(frame, [], {"paused": True, "owner": None}, True)
        self.assertEqual(normal.shape, (544, 640, 3))
        np.testing.assert_array_equal(normal[:480, ::-1], mirrored[:480])
        np.testing.assert_array_equal(frame, original)

    def test_worker_applies_settings_and_preview_commands_with_real_models(self):
        with TemporaryDirectory() as directory:
            root = find_repo_root()
            video = Path(directory) / 'camera.avi'
            metrics = Path(directory) / 'metrics.jsonl'
            preferences = Path(directory) / '.env'
            writer = cv2.VideoWriter(str(video), cv2.VideoWriter_fourcc(*'MJPG'), 10, (320, 240))
            self.assertTrue(writer.isOpened())
            for _ in range(5):
                writer.write(np.zeros((240, 320, 3), np.uint8))
            writer.release()
            output = StringIO()
            channel = IpcChannel(output, StringIO(''))
            commands = iter([
                {"cmd": "pause"}, None,
                {"cmd": "set_confidence", "percent": 35},
                {"cmd": "set_alert_delay", "seconds": 3},
                {"cmd": "toggle_preview"}, None,
                {"cmd": "toggle_mirror"}, {"cmd": "toggle_preview"},
                {"cmd": "resume"}, None,
                {"cmd": "quit"},
            ])
            args = argparse.Namespace(model=root / 'model/yolov8n-face.onnx',
                pose_model=root / 'model/face_landmarker.task', confidence=.5, iou=.45,
                threads=2, video=video, camera=0, width=320, height=240, mirror=True,
                headless=True, compact_preview=True, metrics=metrics, max_frames=None,
                ipc='stdio')
            with patch('no_peek.main.IpcChannel', return_value=channel), \
                 patch.object(channel, 'poll_command', side_effect=lambda: next(commands)), \
                 patch('no_peek.main.save_preferences', side_effect=lambda values: save_preferences(values, preferences)), \
                 patch('no_peek.main.cv2.namedWindow') as opened, \
                 patch('no_peek.main.cv2.resizeWindow'), \
                 patch('no_peek.main.cv2.createTrackbar'), \
                 patch('no_peek.main.cv2.setTrackbarPos'), \
                 patch('no_peek.main.cv2.imshow'), \
                 patch('no_peek.main.cv2.getWindowProperty', return_value=1), \
                 patch('no_peek.main.cv2.waitKey', return_value=255), \
                 patch('no_peek.main.cv2.destroyWindow') as closed, \
                 patch('no_peek.main.cv2.destroyAllWindows'):
                run(args)
            opened.assert_called_once()
            closed.assert_called_once()
            frames = [json.loads(line) for line in metrics.read_text().splitlines() if json.loads(line)['type'] == 'frame']
            self.assertEqual([frame['index'] for frame in frames], [0, 1])
            self.assertEqual(frames[1]['confidence'], .35)
            events = [json.loads(line) for line in output.getvalue().splitlines()]
            self.assertFalse(any(event['type'] == 'error' for event in events))
            settings = [event for event in events if event['type'] == 'settings']
            self.assertTrue(any(event['preview'] for event in settings))
            self.assertEqual(settings[-1], {"type": "settings", "confidence": .35,
                "alert_after_s": 3., "escalated_after_s": 8., "preview": False, "mirror": False})
            self.assertIn('NOPEEK_MIRROR=false', preferences.read_text())
            self.assertTrue(any(event.get('paused') for event in events))
