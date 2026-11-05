"""Uji integrasi kontrol player dengan video lokal; GUI keyboard dimock agar deterministik."""

import argparse
import json
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import cv2
import numpy as np

from no_peek.main import run
from no_peek.source import FrameSource
from test_experiment import sample_face


class PlayerTests(unittest.TestCase):
    """Video fixture sungguhan membuktikan decode, seek, pause dan ekspor timeline."""

    def setUp(self):
        """Tanpa input; buat video sementara 30 frame @10FPS, tanpa mengambil kamera."""
        self.temporary = TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.video = Path(self.temporary.name)/'fixture.avi'
        writer = cv2.VideoWriter(str(self.video), cv2.VideoWriter_fourcc(*'MJPG'), 10., (320, 240))
        self.assertTrue(writer.isOpened())
        for index in range(30):
            writer.write(np.full((240, 320, 3), index*5, np.uint8))
        writer.release()

    def test_source_seek_and_clock(self):
        """Seek ke frame 12 -> frame berikutnya index12 dan timestamp1.2s."""
        source = FrameSource(self.video, 0, 1280, 720)
        try:
            self.assertTrue(source.read()[0])
            self.assertEqual(source.timestamp, 0.)
            source.seek(12)
            self.assertTrue(source.read()[0])
            self.assertEqual(source.index, 12)
            self.assertAlmostEqual(source.timestamp, 1.2)
        finally:
            source.close()

    def test_player_controls_and_metrics(self):
        """Pause tidak mengulang inferensi; step/seek/replay mereset segment dan timer."""
        metrics = Path(self.temporary.name)/'metrics.jsonl'
        config = argparse.Namespace(model=Path('unused.onnx'), confidence=.5, iou=.45,
            threads=2, video=self.video, camera=0, width=1280, height=720,
            mirror=False, headless=False, metrics=metrics, max_frames=None)
        timings = dict(preprocess_ms=1., inference_ms=2., postprocess_ms=1., detector_ms=4.)
        fake_detector = SimpleNamespace(size=(640, 640), confidence=.5, iou=.45,
                                       detect=lambda frame: ([sample_face()], timings))
        # Frame0 -> pause, calibrate, idle, step1, back0, forward to end,
        # restart0, pause, speed+, mirror, reset, quit. No OS keystrokes involved.
        keys = [32, ord('c'), 255, ord('n'), ord('b'), ord('l'), ord('0'),
                32, ord('+'), ord('m'), ord('r'), ord('q')]
        with patch('no_peek.main.FaceDetector', return_value=fake_detector), \
             patch('no_peek.main.cv2.namedWindow'), patch('no_peek.main.cv2.resizeWindow'), \
             patch('no_peek.main.cv2.createTrackbar'), patch('no_peek.main.cv2.setTrackbarPos'), \
             patch('no_peek.main.cv2.imshow'), patch('no_peek.main.cv2.destroyAllWindows'), \
             patch('no_peek.main.cv2.getWindowProperty', return_value=1), \
             patch('no_peek.main.cv2.waitKey', side_effect=keys):
            run(config)
        records = [json.loads(line) for line in metrics.read_text().splitlines()]
        frames = [row for row in records if row['type'] == 'frame']
        self.assertEqual([row['index'] for row in frames], [0, 1, 0, 29, 0])
        self.assertEqual([row['segment'] for row in frames], [0, 0, 1, 2, 3])
        self.assertTrue(all(row['faces'][0]['risk_s'] == 0 for row in frames))
        self.assertEqual(records[-1]['processed_frames'], 5)
        self.assertTrue(any(row.get('action') == 'c' and row['faces'][0]['role'] == 'OWNER' for row in records))

    def test_eof_replay_and_slider(self):
        """EOF berhenti tanpa inferensi ulang; SPACE replay dan slider seek mereset timeline."""
        metrics = Path(self.temporary.name)/'seek.jsonl'
        config = argparse.Namespace(model=Path('unused.onnx'), confidence=.5, iou=.45,
            threads=2, video=self.video, camera=0, width=1280, height=720,
            mirror=False, headless=False, metrics=metrics, max_frames=None)
        timings = dict(preprocess_ms=1., inference_ms=2., postprocess_ms=1., detector_ms=4.)
        fake_detector = SimpleNamespace(size=(640, 640), confidence=.5, iou=.45,
                                       detect=lambda frame: ([sample_face()], timings))
        callback = {}
        keys = iter([ord('l'), 255, 32, 32, 'slider', ord('q')])

        def wait_key(delay):
            """Input jeda UI; output tombol deterministik atau gesture slider 30%."""
            key = next(keys)
            if key == 'slider':
                callback['fn'](300)
                return 255
            return key

        with patch('no_peek.main.FaceDetector', return_value=fake_detector), \
             patch('no_peek.main.cv2.namedWindow'), patch('no_peek.main.cv2.resizeWindow'), \
             patch('no_peek.main.cv2.createTrackbar', side_effect=lambda *args: callback.update(fn=args[-1])), \
             patch('no_peek.main.cv2.setTrackbarPos', side_effect=lambda name, window, value: callback['fn'](value)), \
             patch('no_peek.main.cv2.imshow'), patch('no_peek.main.cv2.destroyAllWindows'), \
             patch('no_peek.main.cv2.getWindowProperty', return_value=1), \
             patch('no_peek.main.cv2.waitKey', side_effect=wait_key):
            run(config)
        records = [json.loads(line) for line in metrics.read_text().splitlines()]
        frames = [row for row in records if row['type'] == 'frame']
        self.assertEqual([row['index'] for row in frames], [0, 29, 0, 9])
        self.assertEqual([row['segment'] for row in frames], [0, 1, 2, 3])


if __name__ == '__main__':
    unittest.main()
