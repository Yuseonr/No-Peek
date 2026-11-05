"""Eksperimen YOLOv8-Face: BGR frame -> box, skor, dan lima landmark.

Format model mengikuti bobot yakhyo/yolov8-face-onnx-inference. Decoder
ditulis untuk raw feature maps dengan DFL 16 bin dan stride 8/16/32.
"""

from dataclasses import dataclass
from pathlib import Path
from time import perf_counter

import cv2
import numpy as np
import onnxruntime as ort


@dataclass
class Face:
    """Data keluaran detektor; seluruh koordinat dalam piksel frame asli."""

    box: np.ndarray  # shape (4,), x1, y1, x2, y2
    score: float
    landmarks: np.ndarray  # shape (5, 2): dua mata, hidung, dua sudut mulut
    pose: dict[str, float] | None = None


def letterbox(frame: np.ndarray, size: tuple[int, int]) -> tuple[np.ndarray, float, tuple[int, int]]:
    """Input BGR HWC dan (width,height); output tensor RGB NCHW, skala, padding kiri/atas."""
    height, width = frame.shape[:2]
    target_w, target_h = size
    scale = min(target_w / width, target_h / height)
    resized_w, resized_h = round(width * scale), round(height * scale)
    left, top = (target_w - resized_w) // 2, (target_h - resized_h) // 2
    canvas = np.full((target_h, target_w, 3), 114, dtype=np.uint8)
    canvas[top:top + resized_h, left:left + resized_w] = cv2.resize(frame, (resized_w, resized_h))
    tensor = cv2.dnn.blobFromImage(canvas, scalefactor=1.0 / 255.0, size=(target_w, target_h), swapRB=True, crop=False)
    return tensor, scale, (left, top)


def suppress_overlaps(boxes: np.ndarray, scores: np.ndarray, threshold: float) -> list[int]:
    """Input box xyxy, skor, ambang IoU; output indeks box yang lolos NMS via OpenCV C++."""
    if boxes.size == 0 or scores.size == 0:
        return []
    widths_heights = np.maximum(boxes[:, 2:] - boxes[:, :2], 0.0)
    bboxes_xywh = np.column_stack((boxes[:, :2], widths_heights)).tolist()
    indices = cv2.dnn.NMSBoxes(bboxes_xywh, scores.tolist(), score_threshold=0.0, nms_threshold=float(threshold))
    if len(indices) == 0:
        return []
    return [int(i) for i in np.asarray(indices).reshape(-1)]


def decode_maps(outputs: list[np.ndarray], confidence: float) -> np.ndarray:
    """Input tiga tensor [1,80,H,W]; output [N,15] berisi xyxy, skor, landmark xy."""
    rows = []
    # Bentuk spatial menentukan stride, sehingga urutan output ONNX tidak diasumsikan.
    for feature in sorted(outputs, key=lambda value: value.shape[-1], reverse=True):
        _, channels, height, width = feature.shape
        if channels != 80:
            raise ValueError(f"Expected 80 channels, got {feature.shape}")
        stride = 8 * (2 ** len(rows))
        vectors = feature[0].transpose(1, 2, 0).reshape(-1, 80)
        scores = 1 / (1 + np.exp(-np.clip(vectors[:, 64], -80, 80)))
        mask = scores >= confidence
        grid = np.stack(np.meshgrid(np.arange(width), np.arange(height)), axis=-1).reshape(-1, 2)
        chosen, anchors = vectors[mask], grid[mask]
        logits = chosen[:, :64].reshape(-1, 4, 16)
        probabilities = np.exp(logits - logits.max(axis=-1, keepdims=True))
        distances = (probabilities / probabilities.sum(axis=-1, keepdims=True)) @ np.arange(16)
        centers = (anchors + 0.5) * stride
        boxes = np.concatenate((centers - distances[:, :2] * stride,
                                centers + distances[:, 2:] * stride), axis=1)
        points = (chosen[:, 65:].reshape(-1, 5, 3)[:, :, :2] * 2 + anchors[:, None, :]) * stride
        rows.append(np.column_stack((boxes, scores[mask], points.reshape(-1, 10))))
    return np.concatenate(rows, axis=0)


class FaceDetector:
    """Pemilik sesi ONNX; inferensi CPU dibuat eksplisit agar mudah direproduksi di Mac."""

    def __init__(self, model: Path, confidence: float = 0.5, iou: float = 0.45, threads: int = 2):
        """Input path model, threshold, jumlah thread; output sesi siap pakai atau error jelas."""
        if not model.is_file():
            raise FileNotFoundError(f"Model tidak ditemukan: {model}")
        options = ort.SessionOptions()
        options.intra_op_num_threads = threads
        options.inter_op_num_threads = 1
        ort.disable_telemetry_events()
        self.session = ort.InferenceSession(str(model), options, providers=["CPUExecutionProvider"])
        model_input = self.session.get_inputs()[0]
        if len(model_input.shape) != 4 or model_input.shape[1] != 3:
            raise ValueError(f"Model harus RGB NCHW: {model_input.shape}")
        # Bobot unduhan memakai dimensi dinamis; 640 adalah resolusi eksperimen awal.
        self.size = tuple(value if isinstance(value, int) else 640 for value in model_input.shape[3:1:-1])
        output_shapes = [item.shape for item in self.session.get_outputs()]
        if len(output_shapes) != 3 or any(len(shape) != 4 or shape[1] != 80 for shape in output_shapes):
            raise ValueError(f"Format model berbeda dari YOLOv8n-Face raw maps: {output_shapes}")
        self.input_name = model_input.name
        self.confidence, self.iou = confidence, iou

    def detect(self, frame: np.ndarray) -> tuple[list[Face], dict[str, float]]:
        """Input frame BGR asli; output wajah di koordinat asli dan waktu tahap dalam ms."""
        start = perf_counter()
        tensor, scale, (left, top) = letterbox(frame, self.size)
        preprocessed = perf_counter()
        outputs = self.session.run(None, {self.input_name: tensor})
        inferred = perf_counter()
        predictions = decode_maps(outputs, self.confidence)
        selected = suppress_overlaps(predictions[:, :4], predictions[:, 4], self.iou)
        faces = []
        height, width = frame.shape[:2]
        for index in selected[:50]:
            row = predictions[index]
            box = (row[:4] - np.array([left, top, left, top])) / scale
            box[[0, 2]] = box[[0, 2]].clip(0, width - 1)
            box[[1, 3]] = box[[1, 3]].clip(0, height - 1)
            points = (row[5:].reshape(5, 2) - (left, top)) / scale
            if box[2] > box[0] and box[3] > box[1] and np.isfinite(points).all():
                faces.append(Face(box, float(row[4]), points))
        end = perf_counter()
        return faces, {"preprocess_ms": (preprocessed-start)*1000,
                       "inference_ms": (inferred-preprocessed)*1000,
                       "postprocess_ms": (end-inferred)*1000,
                       "detector_ms": (end-start)*1000}
