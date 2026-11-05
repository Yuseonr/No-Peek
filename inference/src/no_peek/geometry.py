"""Geometri spasial: estimasi jarak kamera-wajah, zona posisi, orientasi, dan relasi berdampingan (FRIEND)."""

from math import atan2, cos, degrees, radians, tan

import numpy as np

REAL_FACE_WIDTH_CM = 15.0
REAL_IPD_CM = 6.3


def focal_length_px(frame_width: int, fov_deg: float = 60.0) -> float:
    """Input lebar frame piksel dan FoV horizontal derajat; output panjang fokus kamera (piksel)."""
    clamped_fov = max(10.0, min(float(fov_deg), 150.0))
    return (frame_width / 2.0) / tan(radians(clamped_fov / 2.0))


def estimate_distance_cm(
    box: np.ndarray,
    landmarks: np.ndarray | None,
    frame_width: int,
    yaw_deg: float | None = None,
    fov_deg: float = 60.0,
) -> float:
    """Input box xyxy, landmark 5 titik, lebar frame, yaw, FoV; output estimasi jarak wajah (cm)."""
    focal_px = focal_length_px(frame_width, fov_deg)
    box_w = max(1.0, float(box[2] - box[0]))
    dist_box = (REAL_FACE_WIDTH_CM * focal_px) / box_w
    if landmarks is not None and len(landmarks) >= 2:
        eyes = landmarks[:2]
        eye_dist = float(np.linalg.norm(eyes[1] - eyes[0]))
        if eye_dist >= 2.0:
            yaw_rad = radians(abs(float(yaw_deg))) if yaw_deg is not None else 0.0
            corrected_eye_dist = eye_dist / max(cos(yaw_rad), 0.5)
            dist_eyes = (REAL_IPD_CM * focal_px) / corrected_eye_dist
            return round(0.6 * dist_box + 0.4 * dist_eyes, 1)
    return round(dist_box, 1)


def horizontal_zone(box: np.ndarray, frame_width: int) -> str:
    """Input box xyxy dan lebar frame; output zona horizontal LEFT, CENTER, atau RIGHT."""
    center_x = float((box[0] + box[2]) / 2.0)
    if center_x < frame_width / 3.0:
        return "LEFT"
    if center_x > frame_width * 2.0 / 3.0:
        return "RIGHT"
    return "CENTER"


def is_center_zone(box: np.ndarray, frame_width: int, margin: float = 0.25) -> bool:
    """Input box xyxy, lebar frame, batas tepi; output True bila pusat wajah di zona tengah."""
    center_x = float((box[0] + box[2]) / 2.0)
    return (frame_width * margin) <= center_x <= (frame_width * (1.0 - margin))


def is_side_by_side_friend(
    face_box: np.ndarray,
    face_dist_cm: float,
    owner_box: np.ndarray,
    owner_dist_cm: float,
    friend_margin_cm: float = 25.0,
    max_friend_dist_cm: float = 115.0,
) -> bool:
    """Input geometri wajah dan owner; output True bila wajah duduk berdampingan (FRIEND)."""
    if face_dist_cm > max_friend_dist_cm or owner_dist_cm > max_friend_dist_cm:
        return False
    face_w = max(1.0, float(face_box[2] - face_box[0]))
    face_h = max(1.0, float(face_box[3] - face_box[1]))
    owner_w = max(1.0, float(owner_box[2] - owner_box[0]))
    owner_h = max(1.0, float(owner_box[3] - owner_box[1]))

    face_cx = float((face_box[0] + face_box[2]) / 2.0)
    face_cy = float((face_box[1] + face_box[3]) / 2.0)
    owner_cx = float((owner_box[0] + owner_box[2]) / 2.0)
    owner_cy = float((owner_box[1] + owner_box[3]) / 2.0)

    # Harus bersebelahan secara horizontal (tidak menumpuk tepat di atas/belakang kepala owner).
    horizontal_sep = abs(face_cx - owner_cx)
    if horizontal_sep < 0.6 * min(face_w, owner_w):
        return False

    # Posisi vertikal tidak boleh melongok tinggi di atas kepala owner (ciri pengintip berdiri di belakang).
    if face_cy < owner_cy - 0.55 * owner_h:
        return False
    vertical_diff = abs(face_cy - owner_cy)
    if vertical_diff > 0.85 * max(face_h, owner_h):
        return False

    # 1) Jarak hampir sejajar dengan owner.
    if face_dist_cm <= owner_dist_cm + friend_margin_cm or face_dist_cm <= owner_dist_cm * 1.30:
        return True

    # 2) Toleransi ukuran kepala lebih mungil bila duduk sejajar secara vertikal di samping owner.
    vertically_level = vertical_diff <= 0.50 * owner_h
    reasonable_head_ratio = face_w >= 0.60 * owner_w and face_dist_cm <= owner_dist_cm * 1.65
    return vertically_level and reasonable_head_ratio


def intersection_over_union(first: np.ndarray, second: np.ndarray) -> float:
    """Input dua box xyxy; output IoU [0,1] untuk pencocokan track."""
    intersection = np.prod(np.maximum(np.minimum(first[2:], second[2:]) - np.maximum(first[:2], second[:2]), 0))
    areas = np.prod(first[2:] - first[:2]) + np.prod(second[2:] - second[:2])
    return float(intersection / max(areas - intersection, 1e-9))


def orientation(
    points: np.ndarray,
    pose: dict[str, float] | None = None,
    max_yaw: float = 35.0,
    max_pitch: float = 25.0,
    max_roll: float = 35.0,
) -> dict:
    """Input lima landmark xy dan pose 3D opsional; output orientasi MediaPipe atau fallback proxy."""
    eyes = points[:2][np.argsort(points[:2, 0])]
    vector = eyes[1] - eyes[0]
    length = float(np.linalg.norm(vector))
    if length < 3:
        offset, eye_roll, proxy_frontal = None, None, False
    else:
        # Proyeksi hidung pada sumbu mata mengurangi pengaruh kemiringan kepala.
        offset = float(np.dot(points[2] - eyes.mean(axis=0), vector / length) / length)
        eye_roll = degrees(atan2(float(vector[1]), float(vector[0])))
        proxy_frontal = abs(offset) < 0.3 and abs(eye_roll) < max_roll
    if pose is not None and all(k in pose and pose[k] is not None for k in ("yaw_deg", "pitch_deg", "roll_deg")):
        yaw = float(pose["yaw_deg"])
        pitch = float(pose["pitch_deg"])
        roll = float(pose["roll_deg"])
        frontal = abs(yaw) < max_yaw and abs(pitch) < max_pitch and abs(roll) < max_roll
        return {
            "yaw_deg": yaw,
            "pitch_deg": pitch,
            "roll_deg": roll,
            "nose_offset": offset,
            "pose_source": "mediapipe",
            "frontal_proxy": frontal,
        }
    return {
        "yaw_deg": None,
        "pitch_deg": None,
        "roll_deg": eye_roll,
        "nose_offset": offset,
        "pose_source": "five_point_proxy",
        "frontal_proxy": proxy_frontal,
    }
