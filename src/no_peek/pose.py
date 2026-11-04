"""Estimasi head pose 3D (yaw, pitch, roll) menggunakan MediaPipe Face Landmarker."""

from math import atan2, degrees, hypot, radians
from pathlib import Path
from time import perf_counter

import cv2
import mediapipe as mp
import numpy as np

from no_peek.detector import Face

DEFAULT_POSE_MODEL = Path(__file__).resolve().parents[2] / "model/face_landmarker.task"


def matrix_to_angles(matrix: np.ndarray) -> dict[str, float] | None:
    """Input matriks transformasi 4x4/3x3 MediaPipe; output yaw, pitch, roll dalam derajat."""
    arr = np.asarray(matrix, dtype=np.float64)
    if arr.shape not in ((4, 4), (3, 3)) or not np.isfinite(arr).all():
        return None
    rotation = arr[:3, :3]
    norms = np.linalg.norm(rotation, axis=0, keepdims=True)
    if np.any(norms < 1e-6):
        return None
    rotation = rotation / norms
    # Dekomposisi R = Ry(yaw) * Rx(pitch) * Rz(roll) pada ruang metrik wajah MediaPipe (+X kanan, +Y atas, +Z kamera).
    forward_x, forward_y, forward_z = float(rotation[0, 2]), float(rotation[1, 2]), float(rotation[2, 2])
    yaw_deg = degrees(atan2(forward_x, forward_z))
    pitch_deg = degrees(atan2(forward_y, hypot(forward_x, forward_z)))
    roll_deg = degrees(atan2(-float(rotation[1, 0]), float(rotation[1, 1])))
    return {"yaw_deg": yaw_deg, "pitch_deg": pitch_deg, "roll_deg": roll_deg}


def euler_to_matrix(yaw_deg: float, pitch_deg: float, roll_deg: float) -> np.ndarray:
    """Input sudut yaw, pitch, roll (derajat); output matriks transformasi homogen 4x4."""
    yaw, pitch, roll = radians(yaw_deg), -radians(pitch_deg), -radians(roll_deg)
    cy, sy = np.cos(yaw), np.sin(yaw)
    cp, sp = np.cos(pitch), np.sin(pitch)
    cr, sr = np.cos(roll), np.sin(roll)
    ry = np.array([[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]])
    rx = np.array([[1.0, 0.0, 0.0], [0.0, cp, -sp], [0.0, sp, cp]])
    rz = np.array([[cr, -sr, 0.0], [sr, cr, 0.0], [0.0, 0.0, 1.0]])
    matrix = np.eye(4, dtype=np.float64)
    matrix[:3, :3] = ry @ rx @ rz
    return matrix


def crop_face(frame: np.ndarray, box: np.ndarray, margin: float = 0.25) -> np.ndarray:
    """Input frame BGR dan box xyxy; output crop persegi RGB supaya aspek rasio 1:1 terjaga."""
    height, width = frame.shape[:2]
    x1, y1, x2, y2 = (float(v) for v in box)
    box_w, box_h = max(1.0, x2 - x1), max(1.0, y2 - y1)
    cx, cy = (x1 + x2) / 2.0, (y1 + y2) / 2.0
    half = max(16, round(max(box_w, box_h) * (0.5 + margin)))
    left, top = round(cx) - half, round(cy) - half
    right, bottom = round(cx) + half, round(cy) + half
    pad_left = max(0, -left)
    pad_top = max(0, -top)
    pad_right = max(0, right - width)
    pad_bottom = max(0, bottom - height)
    cropped = frame[max(0, top):min(height, bottom), max(0, left):min(width, right)]
    if any((pad_left, pad_top, pad_right, pad_bottom)):
        cropped = cv2.copyMakeBorder(
            cropped, pad_top, pad_bottom, pad_left, pad_right,
            cv2.BORDER_CONSTANT, value=(114, 114, 114),
        )
    return cv2.cvtColor(cropped, cv2.COLOR_BGR2RGB)


class HeadPoseEstimator:
    """Tahap kedua pipeline: estimasi head pose 3D per wajah menggunakan MediaPipe Tasks."""

    def __init__(self, model: Path = DEFAULT_POSE_MODEL):
        """Input path model .task MediaPipe; output estimator siap pakai atau error jelas."""
        if not model.is_file():
            raise FileNotFoundError(f"Model pose tidak ditemukan: {model}")
        options = mp.tasks.vision.FaceLandmarkerOptions(
            base_options=mp.tasks.BaseOptions(model_asset_path=str(model)),
            running_mode=mp.tasks.vision.RunningMode.IMAGE,
            num_faces=1,
            min_face_detection_confidence=0.3,
            min_face_presence_confidence=0.3,
            output_facial_transformation_matrixes=True,
        )
        self.landmarker = mp.tasks.vision.FaceLandmarker.create_from_options(options)
        self.model = model

    def estimate(self, frame: np.ndarray, faces: list[Face], min_area_ratio: float = 0.0015) -> float:
        """Input frame BGR dan daftar Face; isi face.pose secara in-place dan kembalikan waktu ms."""
        start = perf_counter()
        height, width = frame.shape[:2]
        frame_area = max(1, width * height)
        for face in faces:
            area_ratio = float(np.prod(np.maximum(face.box[2:] - face.box[:2], 0)) / frame_area)
            if area_ratio < min_area_ratio:
                continue
            rgb_crop = crop_face(frame, face.box)
            mp_image = mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb_crop)
            result = self.landmarker.detect(mp_image)
            if result.facial_transformation_matrixes:
                face.pose = matrix_to_angles(result.facial_transformation_matrixes[0])
        return (perf_counter() - start) * 1000.0

    def close(self) -> None:
        """Tanpa input/output; tutup resource native MediaPipe FaceLandmarker."""
        self.landmarker.close()
