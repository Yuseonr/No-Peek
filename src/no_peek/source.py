"""Akuisisi sumber video dan webcam; tidak melakukan inferensi atau menggambar UI."""

from pathlib import Path
import sys
import time

import cv2
import numpy as np


class FrameSource:
    """Video memakai waktu media; webcam memakai jam monotonik sejak sumber dibuka."""

    def __init__(self, video: Path | None, camera: int, width: int, height: int):
        """Input path video atau index kamera dan resolusi; output capture dengan metadata."""
        self.is_video = video is not None
        if video is not None and not video.is_file():
            raise FileNotFoundError(f"Video tidak ditemukan: {video}")
        self.label = video.name if video else f"Webcam {camera}"
        if self.is_video:
            self.capture = cv2.VideoCapture(str(video))
        else:
            backend = cv2.CAP_AVFOUNDATION if sys.platform == "darwin" else cv2.CAP_ANY
            self.capture = cv2.VideoCapture(camera, backend)
            self.capture.set(cv2.CAP_PROP_FRAME_WIDTH, width)
            self.capture.set(cv2.CAP_PROP_FRAME_HEIGHT, height)
            self.capture.set(cv2.CAP_PROP_FPS, 30)
        if not self.capture.isOpened():
            self.capture.release()
            raise RuntimeError("Sumber tidak dapat dibuka. Untuk webcam Mac, izinkan Camera untuk Terminal/editor di System Settings > Privacy & Security.")
        reported_fps = self.capture.get(cv2.CAP_PROP_FPS)
        self.fps = reported_fps if np.isfinite(reported_fps) and reported_fps > 0 else 30.0
        self.frame_count = max(0, int(self.capture.get(cv2.CAP_PROP_FRAME_COUNT))) if self.is_video else 0
        self.index = -1
        self.started = time.monotonic()
        self.timestamp = 0.0

    def read(self) -> tuple[bool, np.ndarray | None]:
        """Tanpa input; output (berhasil, frame BGR), memperbarui index dan timestamp detik."""
        ok, frame = self.capture.read()
        if ok:
            self.index += 1
            self.timestamp = self.index / self.fps if self.is_video else time.monotonic() - self.started
        return ok, frame

    def seek(self, index: int) -> None:
        """Input index frame video; set posisi baca berikutnya. Webcam tidak bisa seek."""
        if not self.is_video:
            return
        index = max(0, min(index, max(0, self.frame_count - 1)))
        if not self.capture.set(cv2.CAP_PROP_POS_FRAMES, index):
            raise RuntimeError("Backend video tidak mendukung seek untuk file ini.")
        self.index = index - 1

    def close(self) -> None:
        """Tanpa input/output; lepaskan perangkat/file, termasuk saat ada exception."""
        self.capture.release()
