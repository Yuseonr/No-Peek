"""Analisis eksperimen: tracking spasial, proxy orientasi, dan timer per wajah.

ID bukan identitas biometrik. Sesudah owner hilang melebihi grace period,
kalibrasi dibatalkan agar ID baru tidak otomatis dianggap owner/observer.
"""

from dataclasses import dataclass
from math import atan2, degrees

import numpy as np

from no_peek.detector import Face


def orientation(points: np.ndarray) -> dict:
    """Input lima landmark xy; output roll garis mata dan proxy frontal (bukan gaze/yaw/pitch)."""
    eyes = points[:2][np.argsort(points[:2, 0])]
    vector = eyes[1] - eyes[0]
    length = float(np.linalg.norm(vector))
    if length < 3:
        return {"roll_deg": None, "nose_offset": None, "frontal_proxy": False}
    # Proyeksi hidung pada sumbu mata mengurangi pengaruh kemiringan kepala.
    offset = float(np.dot(points[2] - eyes.mean(axis=0), vector / length) / length)
    roll = degrees(atan2(float(vector[1]), float(vector[0])))
    return {"roll_deg": roll, "nose_offset": offset,
            "frontal_proxy": abs(offset) < 0.3 and abs(roll) < 35}


def intersection_over_union(first: np.ndarray, second: np.ndarray) -> float:
    """Input dua box xyxy; output IoU [0,1] untuk pencocokan track."""
    intersection = np.prod(np.maximum(np.minimum(first[2:], second[2:]) - np.maximum(first[:2], second[:2]), 0))
    areas = np.prod(first[2:] - first[:2]) + np.prod(second[2:] - second[:2])
    return float(intersection / max(areas - intersection, 1e-9))


@dataclass
class Track:
    """State per lintasan sementara; waktu memakai detik pada timeline sumber."""

    identifier: int
    face: Face
    last_seen: float
    risk_since: float | None = None
    state: str = "MONITORING"


class Analyzer:
    """Eksperimen aturan 2s/7s, grace period 250ms, dan kalibrasi owner eksplisit."""

    def __init__(self, grace: float = 0.25):
        """Input toleransi missed detection (detik); output state tracking kosong."""
        self.grace = grace
        self.reset()

    def reset(self) -> None:
        """Tanpa input; hapus track, kalibrasi, dan episode saat reset/seek/replay."""
        self.tracks: dict[int, Track] = {}
        self.next_id = 1
        self.owner: int | None = None
        self.calibrated = False
        self.events: list[dict] = []

    def calibrate(self, visible_ids: list[int]) -> str:
        """Input ID yang terlihat sekarang; output pesan kalibrasi (tepat satu wajah wajib)."""
        if len(visible_ids) != 1:
            return "Kalibrasi gagal: harus tepat 1 wajah terlihat."
        self.owner = visible_ids[0]
        self.calibrated = True
        for track in self.tracks.values():
            track.risk_since, track.state = None, "MONITORING"
        return f"Owner = ID {self.owner}. Tracking spasial, bukan face recognition."

    def update(self, faces: list[Face], now: float, shape: tuple) -> list[dict]:
        """Input deteksi, timestamp sumber, shape frame; output analisis wajah yang terlihat.

        Matching satu-ke-satu berdasarkan IoU; track yang tak terlihat tidak
        digambar. Risiko berlanjut hanya selama missed detection <= grace.
        """
        self.events = []
        expired = [key for key, track in self.tracks.items() if now-track.last_seen > self.grace]
        for key in expired:
            del self.tracks[key]
            if key == self.owner:
                self.calibrated, self.owner = False, None
        candidates = sorted(
            ((intersection_over_union(track.face.box, face.box), key, index)
             for key, track in self.tracks.items() for index, face in enumerate(faces)),
            reverse=True,
        )
        assignments, used = {}, set()
        for overlap, key, index in candidates:
            if overlap >= 0.25 and key not in used and index not in assignments:
                assignments[index] = key
                used.add(key)
        result = []
        height, width = shape[:2]
        for index, face in enumerate(faces):
            key = assignments.get(index)
            if key is None:
                key = self.next_id
                self.next_id += 1
                self.tracks[key] = Track(key, face, now)
            track = self.tracks[key]
            track.face, track.last_seen = face, now
            pose = orientation(face.landmarks)
            area_ratio = float(np.prod(face.box[2:]-face.box[:2]) / (width*height))
            role = "OWNER" if key == self.owner else "OBSERVER" if self.calibrated else "UNCALIBRATED"
            qualifies = role == "OBSERVER" and pose["frontal_proxy"] and area_ratio >= 0.0015
            previous = track.state
            if qualifies:
                if track.risk_since is None:
                    track.risk_since = now
                elapsed = now - track.risk_since
                track.state = "ESCALATED" if elapsed >= 7 else "ALERT" if elapsed >= 2 else "PENDING"
            else:
                track.risk_since, track.state, elapsed = None, "MONITORING", 0.0
            if track.state != previous and track.state in ("ALERT", "ESCALATED"):
                self.events.append({"time_s": now, "id": key, "event": track.state})
            center_x = float((face.box[0] + face.box[2]) / 2)
            position = "LEFT" if center_x < width/3 else "RIGHT" if center_x > width*2/3 else "CENTER"
            result.append({"id": key, "role": role, "state": track.state, "risk_s": elapsed,
                           "position": position, "area_ratio": area_ratio, "confidence": face.score,
                           "box": face.box.tolist(), "landmarks": face.landmarks.tolist(), **pose})
        return result
