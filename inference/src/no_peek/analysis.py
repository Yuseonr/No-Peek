"""Analisis eksperimen: tracking spasial, auto-kalibrasi owner, klasifikasi FRIEND/OBSERVER, dan timer risiko."""

from dataclasses import dataclass

import numpy as np

from no_peek.detector import Face
from no_peek.geometry import (
    estimate_distance_cm,
    horizontal_zone,
    intersection_over_union,
    is_center_zone,
    is_side_by_side_friend,
    orientation,
)

__all__ = ["Analyzer", "Track", "intersection_over_union", "orientation"]


@dataclass
class Track:
    """State per lintasan sementara; waktu memakai detik pada timeline sumber."""

    identifier: int
    face: Face
    last_seen: float
    distance_cm: float = 0.0
    risk_since: float | None = None
    state: str = "MONITORING"


class Analyzer:
    """Aturan 2s/7s, grace 250ms, auto/manual kalibrasi owner, dan klasifikasi berdampingan (FRIEND)."""

    def __init__(
        self,
        grace: float = 0.25,
        max_yaw: float = 35.0,
        max_pitch: float = 25.0,
        max_roll: float = 35.0,
        auto_calibrate: bool = True,
        auto_calibrate_s: float = 0.6,
        fov_deg: float = 60.0,
        friend_margin_cm: float = 25.0,
        max_friend_dist_cm: float = 115.0,
    ):
        """Input parameter tracking, sudut, auto-kalibrasi, dan jarak; output state siap pakai."""
        self.grace = grace
        self.max_yaw = max_yaw
        self.max_pitch = max_pitch
        self.max_roll = max_roll
        self.auto_calibrate = auto_calibrate
        self.auto_calibrate_s = auto_calibrate_s
        self.fov_deg = fov_deg
        self.friend_margin_cm = friend_margin_cm
        self.max_friend_dist_cm = max_friend_dist_cm
        self.reset()

    def reset(self) -> None:
        """Tanpa input; hapus track, kalibrasi, dan episode saat reset/seek/replay."""
        self.tracks: dict[int, Track] = {}
        self.next_id = 1
        self.owner: int | None = None
        self.calibrated = False
        self.calibration_mode: str | None = None
        self.candidate_id: int | None = None
        self.candidate_since: float | None = None
        self.events: list[dict] = []

    def calibrate(self, visible_ids: list[int]) -> str:
        """Input ID yang terlihat sekarang; output pesan kalibrasi manual (tepat satu wajah wajib)."""
        if len(visible_ids) != 1:
            return "Kalibrasi gagal: harus tepat 1 wajah terlihat."
        self.owner = visible_ids[0]
        self.calibrated = True
        self.calibration_mode = "MANUAL"
        self.candidate_id, self.candidate_since = None, None
        for track in self.tracks.values():
            track.risk_since, track.state = None, "MONITORING"
        return f"Owner = ID {self.owner} (MANUAL). Tracking spasial, bukan face recognition."

    def toggle_auto_calibrate(self) -> str:
        """Tanpa input; aktifkan/nonaktifkan auto-kalibrasi dan kembalikan pesan status."""
        self.auto_calibrate = not self.auto_calibrate
        self.candidate_id, self.candidate_since = None, None
        state_label = "ON" if self.auto_calibrate else "OFF"
        return f"Auto-calibrate = {state_label}."

    def _try_auto_calibrate(
        self,
        visible_meta: list[tuple[int, Face, dict, float]],
        now: float,
        width: int,
    ) -> None:
        """Input metadata wajah terlihat; kunci OWNER otomatis bila kandidat tengah & dekat stabil."""
        if self.calibrated or not self.auto_calibrate:
            self.candidate_id, self.candidate_since = None, None
            return
        eligible = []
        for key, face, pose, dist_cm in visible_meta:
            if pose["frontal_proxy"] and dist_cm <= self.max_friend_dist_cm and is_center_zone(face.box, width):
                center_x = float((face.box[0] + face.box[2]) / 2.0)
                score = dist_cm * (1.0 + abs(center_x - width / 2.0) / max(1, width))
                eligible.append((score, key))
        if not eligible:
            self.candidate_id, self.candidate_since = None, None
            return
        _, best_key = min(eligible)
        if self.candidate_id != best_key or self.candidate_since is None:
            self.candidate_id = best_key
            self.candidate_since = now
        if now - self.candidate_since >= self.auto_calibrate_s:
            self.owner = best_key
            self.calibrated = True
            self.calibration_mode = "AUTO"
            self.candidate_id, self.candidate_since = None, None
            for track in self.tracks.values():
                track.risk_since, track.state = None, "MONITORING"
            self.events.append({"time_s": now, "id": best_key, "event": "AUTO_CALIBRATED"})

    def update(self, faces: list[Face], now: float, shape: tuple) -> list[dict]:
        """Input deteksi, timestamp sumber, shape frame; output analisis wajah yang terlihat."""
        self.events = []
        expired = [key for key, track in self.tracks.items() if now - track.last_seen > self.grace]
        for key in expired:
            del self.tracks[key]
            if key == self.owner:
                self.calibrated, self.owner, self.calibration_mode = False, None, None

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

        height, width = shape[:2]
        visible_meta: list[tuple[int, Face, dict, float]] = []
        for index, face in enumerate(faces):
            key = assignments.get(index)
            if key is None:
                key = self.next_id
                self.next_id += 1
                self.tracks[key] = Track(key, face, now)
            track = self.tracks[key]
            pose = orientation(face.landmarks, face.pose, self.max_yaw, self.max_pitch, self.max_roll)
            dist_cm = estimate_distance_cm(face.box, face.landmarks, width, pose["yaw_deg"], self.fov_deg)
            track.face, track.last_seen, track.distance_cm = face, now, dist_cm
            visible_meta.append((key, face, pose, dist_cm))

        self._try_auto_calibrate(visible_meta, now, width)
        owner_track = self.tracks.get(self.owner) if self.calibrated and self.owner is not None else None

        result = []
        for key, face, pose, dist_cm in visible_meta:
            track = self.tracks[key]
            area_ratio = float(np.prod(face.box[2:] - face.box[:2]) / (width * height))
            if key == self.owner:
                role = "OWNER"
            elif not self.calibrated:
                role = "UNCALIBRATED"
            elif owner_track is not None and is_side_by_side_friend(
                face.box, dist_cm, owner_track.face.box, owner_track.distance_cm,
                self.friend_margin_cm, self.max_friend_dist_cm,
            ):
                role = "FRIEND"
            else:
                role = "OBSERVER"

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
            position = horizontal_zone(face.box, width)
            result.append({
                "id": key,
                "role": role,
                "state": track.state,
                "risk_s": elapsed,
                "distance_cm": dist_cm,
                "position": position,
                "area_ratio": area_ratio,
                "confidence": face.score,
                "box": face.box.tolist(),
                "landmarks": face.landmarks.tolist(),
                **pose,
            })
        return result
