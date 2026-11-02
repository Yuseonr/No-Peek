"""Gambar preview eksperimen dan panel data dengan OpenCV HighGUI."""

import cv2
import numpy as np


COLORS = {"MONITORING": (180, 180, 180), "OWNER": (100, 230, 90),
          "PENDING": (0, 210, 255), "ALERT": (30, 80, 255), "ESCALATED": (180, 60, 255)}
POINT_COLORS = [(255, 150, 70), (70, 170, 255), (100, 240, 240), (220, 100, 220), (220, 220, 100)]


def text(image: np.ndarray, message: str, xy: tuple[int, int], color=(220, 225, 235), scale=0.45) -> None:
    """Input canvas BGR, pesan ASCII, posisi; gambar tulisan ke canvas secara in-place."""
    cv2.putText(image, message, xy, cv2.FONT_HERSHEY_SIMPLEX, scale, color, 1, cv2.LINE_AA)


def render(frame: np.ndarray, observations: list[dict], info: dict, mirror: bool) -> np.ndarray:
    """Input frame/data/metrik; output canvas preview+panel. Koordinat data tetap frame asli."""
    height, width = frame.shape[:2]
    display_w, display_h = 900, 620
    factor = min(display_w/width, display_h/height)
    resized_w, resized_h = round(width*factor), round(height*factor)
    view = cv2.resize(frame, (resized_w, resized_h))
    if mirror:
        view = cv2.flip(view, 1)
    canvas = np.full((850, 1400, 3), (22, 25, 31), np.uint8)
    text(canvas, "NO-PEEK / YOLO-FACE EXPERIMENT", (15, 27), scale=0.68)
    text(canvas, f"{info['source']} | {'PAUSED' if info['paused'] else 'PLAYING'} | {info['speed']:.2f}x | mirror={mirror}", (15, 50))
    for observation in observations:
        box = np.asarray(observation["box"]).reshape(2, 2) * factor
        points = np.asarray(observation["landmarks"]) * factor
        if mirror:
            box[:, 0] = resized_w - 1 - box[:, 0]
            points[:, 0] = resized_w - 1 - points[:, 0]
        box = np.sort(box, axis=0).astype(int)
        color = COLORS["OWNER"] if observation["role"] == "OWNER" else COLORS[observation["state"]]
        cv2.rectangle(view, tuple(box[0]), tuple(box[1]), color, 2)
        role = "UNCAL" if observation['role'] == 'UNCALIBRATED' else observation['role']
        label = f"#{observation['id']} {role} {observation['confidence']:.2f}"
        label_w = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, .45, 1)[0][0]
        label_x = max(0, min(int(box[0, 0]), resized_w-label_w-4))
        label_y = max(15, int(box[0, 1])-8)
        cv2.rectangle(view, (label_x, label_y-13), (label_x+label_w+3, label_y+3), (25, 25, 25), -1)
        text(view, label, (label_x+1, label_y), color)
        for point, point_color in zip(points.astype(int), POINT_COLORS):
            cv2.circle(view, tuple(point), 3, point_color, -1)
        eyes = points[:2].astype(int)
        cv2.line(view, tuple(eyes[0]), tuple(eyes[1]), (255, 200, 100), 1)
    canvas[65:65+resized_h, 10:10+resized_w] = view
    x, y = 930, 85
    lines = [f"Frame {info['index']} | media/source {info['time_s']:.2f}s",
             f"Actual capture: {width}x{height} @ {info['source_fps']:.1f} FPS",
             f"Processing: {info['processing_fps']:.1f} FPS (excl. playback wait)",
             f"Inference: {info['inference_ms']:.1f} ms",
             f"Detector total: {info['detector_ms']:.1f} ms",
             f"Inference p50/p95: {info['p50_ms']:.1f}/{info['p95_ms']:.1f} ms",
             f"Faces: {len(observations)} | owner: {info['owner']}",
             f"Confidence >= {info['confidence']:.2f} | NMS {info['iou']:.2f}",
             f"CPU / ONNX | input {info['input_size']} | threads {info['threads']}",
             "", "ORIENTATION = FIVE-POINT PROXY",
             "Roll = eye-line tilt in original frame.",
             "Yaw/pitch/gaze: unavailable from detector.",
             "No ground truth: accuracy/recall not computed."]
    for line in lines:
        text(canvas, line, (x, y))
        y += 23
    # Panel dibatasi tiga wajah agar data tidak menutupi kontrol. Semua wajah tetap digambar/log.
    for observation in observations[:3]:
        color = COLORS["OWNER"] if observation["role"] == "OWNER" else COLORS[observation["state"]]
        position = observation["position"]
        if mirror:
            position = {"LEFT": "RIGHT", "RIGHT": "LEFT", "CENTER": "CENTER"}[position]
        roll = observation["roll_deg"]
        roll_label = "N/A" if roll is None else f"{roll:.1f} deg"
        offset = observation["nose_offset"]
        offset_label = "N/A" if offset is None else f"{offset:+.2f}"
        details = [f"#{observation['id']} {observation['role']} | {observation['state']}",
                   f"{position} | risk {observation['risk_s']:.2f}s | area {100*observation['area_ratio']:.2f}%",
                   f"roll {roll_label} | nose offset {offset_label}",
                   f"frontal proxy: {observation['frontal_proxy']} | conf {observation['confidence']:.3f}",
                   "xyxy: " + ", ".join(f"{value:.0f}" for value in observation['box'])]
        for line in details:
            text(canvas, line, (x, y), color)
            y += 20
        y += 7
    states = {item["state"] for item in observations}
    status = next((state for state in ("ESCALATED", "ALERT", "PENDING") if state in states), "MONITORING")
    if info["owner"] is None:
        status = "UNCALIBRATED - press C when exactly one face is visible"
    text(canvas, status, (15, 720), COLORS.get(status, (220, 220, 220)), 0.65)
    text(canvas, info["message"][:105], (15, 750), (120, 220, 240))
    text(canvas, "C calibrate | R reset | M mirror | SPACE pause | N next frame | B previous frame | Q/ESC quit", (15, 782))
    text(canvas, "VIDEO: J/L -/+5s | 0 restart | +/- playback speed | Position slider seeks and resets tracking", (15, 807))
    text(canvas, "Local processing. No images saved. Potential observer != proven shoulder surfing. No unattended classification.", (15, 831))
    return canvas
