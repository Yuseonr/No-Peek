"""Satu entry point eksperimen: webcam/video -> deteksi -> analisis -> preview/log."""

import argparse
from collections import deque
import json
from pathlib import Path
import sys
import time

import cv2
import numpy as np

from no_peek.analysis import Analyzer
from no_peek.config import AppConfig
from no_peek.detector import FaceDetector
from no_peek.ipc import IpcChannel, build_state_payload
from no_peek.pose import DEFAULT_POSE_MODEL, HeadPoseEstimator
from no_peek.preview import render
from no_peek.source import FrameSource


def _default_yolo_model() -> Path:
    root_candidate = Path(__file__).resolve().parents[3] / "model/yolov8n-face.onnx"
    if root_candidate.is_file():
        return root_candidate
    return Path(__file__).resolve().parents[2] / "model/yolov8n-face.onnx"


def arguments() -> argparse.Namespace:
    """Input argumen terminal; output konfigurasi tervalidasi untuk satu sesi eksperimen."""
    cfg = AppConfig.from_env()
    parser = argparse.ArgumentParser(description="No-Peek: eksperimen webcam/video YOLOv8-Face + MediaPipe Head Pose lokal")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--video", type=Path, help="File video lokal; tanpa opsi ini gunakan webcam")
    source.add_argument("--camera", type=int, default=cfg.camera_index, help="Index webcam (default dari .env)")
    parser.add_argument("--model", type=Path, default=cfg.yolo_model)
    parser.add_argument("--pose-model", type=Path, default=cfg.pose_model, help="Model MediaPipe Face Landmarker (.task)")
    parser.add_argument("--confidence", type=float, default=cfg.confidence)
    parser.add_argument("--iou", type=float, default=cfg.iou)
    parser.add_argument("--max-yaw", type=float, default=cfg.max_yaw, help="Ambang mutlak yaw derajat untuk menghadap layar (default 35)")
    parser.add_argument("--max-pitch", type=float, default=cfg.max_pitch, help="Ambang mutlak pitch derajat untuk menghadap layar (default 25)")
    parser.add_argument("--max-roll", type=float, default=cfg.max_roll, help="Ambang mutlak roll derajat (default 35)")
    parser.add_argument("--fov", type=float, default=cfg.fov_deg, help="FoV horizontal kamera derajat untuk estimasi jarak (default 60)")
    parser.add_argument("--friend-margin-cm", type=float, default=cfg.friend_margin_cm, help="Toleransi jarak belakang owner (cm) untuk role FRIEND (default 25)")
    parser.add_argument("--auto-calibrate", action=argparse.BooleanOptionalAction, default=cfg.auto_calibrate, help="Auto-kalibrasi owner di tengah layar (default True)")
    parser.add_argument("--threads", type=int, default=cfg.onnx_threads, help="Thread CPU ONNX; default 2 untuk baseline Mac")
    parser.add_argument("--width", type=int, default=cfg.camera_width)
    parser.add_argument("--height", type=int, default=cfg.camera_height)
    parser.add_argument("--mirror", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--ipc", choices=["none", "stdio", "tcp"], default=cfg.ipc_mode, help="Mode kanal IPC ke Rust GUI")
    parser.add_argument("--ipc-heartbeat-s", type=float, default=cfg.ipc_heartbeat_s)
    parser.add_argument("--text-alert", type=str, default=cfg.text_alert)
    parser.add_argument("--text-escalated", type=str, default=cfg.text_escalated)
    parser.add_argument("--metrics", type=Path, help="JSONL per frame; opt-in, tidak menyimpan gambar")
    parser.add_argument("--headless", action="store_true", default=None, help="Tanpa window, untuk evaluasi batch video atau prod")
    parser.add_argument("--max-frames", type=int, help="Batasi frame yang diproses untuk smoke test")
    args = parser.parse_args()
    if args.headless is None:
        args.headless = cfg.headless
    if not 0 < args.confidence <= 1 or not 0 < args.iou <= 1:
        parser.error("confidence dan iou harus >0 dan <=1")
    if min(args.threads, args.width, args.height, args.max_yaw, args.max_pitch, args.max_roll, args.fov, args.friend_margin_cm) <= 0 or (args.max_frames is not None and args.max_frames <= 0):
        parser.error("threads, width, height, max-yaw, max-pitch, max-roll, fov, friend-margin-cm, max-frames harus positif")
    if args.camera < 0:
        parser.error("camera harus >=0")
    return args


def write_record(stream, record: dict) -> None:
    """Input file JSONL opsional dan dict; tulis/flush satu record tanpa data gambar."""
    if stream is not None:
        stream.write(json.dumps(record, allow_nan=False) + "\n")
        stream.flush()


def run(args: argparse.Namespace) -> None:
    """Input konfigurasi CLI; kelola sesi, render preview, playback, dan cleanup deterministik."""
    pose_model = getattr(args, "pose_model", DEFAULT_POSE_MODEL)
    max_yaw = getattr(args, "max_yaw", 35.0)
    max_pitch = getattr(args, "max_pitch", 25.0)
    max_roll = getattr(args, "max_roll", 35.0)
    fov_deg = getattr(args, "fov", 60.0)
    friend_margin_cm = getattr(args, "friend_margin_cm", 25.0)
    auto_calibrate = getattr(args, "auto_calibrate", True)

    ipc_mode = getattr(args, "ipc", "none")
    use_ipc = ipc_mode in ("stdio", "tcp")
    ipc_channel: IpcChannel | None = None
    if use_ipc:
        ipc_channel = IpcChannel()
        ipc_channel.start_reader()

    def log(msg: str) -> None:
        if use_ipc and ipc_mode == "stdio":
            sys.stderr.write(str(msg) + "\n")
            sys.stderr.flush()
        else:
            print(msg)

    detector = FaceDetector(args.model, args.confidence, args.iou, args.threads)
    pose_estimator = HeadPoseEstimator(pose_model)
    source = None
    metrics = None
    window = "No-Peek experiment"
    analyzer = Analyzer(
        max_yaw=max_yaw,
        max_pitch=max_pitch,
        max_roll=max_roll,
        auto_calibrate=auto_calibrate,
        fov_deg=fov_deg,
        friend_margin_cm=friend_margin_cm,
    )
    history: deque[float] = deque(maxlen=300)
    paused, eof, mirror = False, False, args.mirror if args.mirror is not None else args.video is None
    speed, processed, segment = 1.0, 0, 0
    requested_seek = None
    updating_slider = True
    message = "Auto-kalibrasi aktif (duduk di tengah). Tekan C untuk manual, A toggle auto."

    def on_position(value: int) -> None:
        """Input posisi slider 0..1000; simpan permintaan seek dari pengguna untuk loop utama."""
        nonlocal requested_seek
        if not updating_slider and source is not None and source.frame_count > 1:
            requested_seek = round(value / 1000 * (source.frame_count - 1))

    def seek(index: int) -> None:
        """Input index tujuan; pindahkan video dan reset analisis supaya timer tidak melintasi lompatan."""
        nonlocal eof, segment, message
        source.seek(index)
        analyzer.reset()
        history.clear()
        segment += 1
        eof = False
        message = "Seek/replay: tracking dan timer direset; kalibrasi ulang dengan C."
        write_record(metrics, {"type": "seek", "segment": segment, "target_frame": index})

    try:
        if args.metrics:
            args.metrics.parent.mkdir(parents=True, exist_ok=True)
            metrics = args.metrics.open("x", encoding="utf-8")
        source = FrameSource(args.video, args.camera, args.width, args.height)
        write_record(metrics, {"type": "session", "model": str(args.model), "pose_model": str(pose_model),
                              "source": source.label, "confidence": args.confidence, "iou": args.iou,
                              "threads": args.threads, "provider": "CPUExecutionProvider",
                              "orientation": "mediapipe_head_pose", "max_yaw": max_yaw,
                              "max_pitch": max_pitch, "max_roll": max_roll,
                              "source_fps": source.fps, "clock": "media" if source.is_video else "monotonic"})
        if ipc_channel:
            ipc_channel.send({
                "type": "ready",
                "version": "0.1.0",
                "source": source.label,
                "headless": bool(args.headless),
            })
        if not args.headless:
            # HighGUI dan input keyboard tetap di main thread untuk backend Cocoa macOS.
            cv2.namedWindow(window, cv2.WINDOW_NORMAL)
            cv2.resizeWindow(window, 1400, 850)
            if source.is_video and source.frame_count > 1:
                cv2.createTrackbar("Position", window, 0, 1000, on_position)
            updating_slider = False
        log(f"Source={source.label}; CPU threads={args.threads}; model input={detector.size}; pose=MediaPipe")
        frame, observations, info = None, [], {}
        needs_frame = True
        processing_ms = 0.0
        while True:
            loop_started = time.perf_counter()
            if requested_seek is not None:
                seek(requested_seek)
                requested_seek, needs_frame = None, True
            if needs_frame:
                ok, next_frame = source.read()
                if not ok:
                    if frame is None:
                        raise RuntimeError("Sumber tidak memiliki frame yang dapat didekode.")
                    if args.headless or not source.is_video:
                        if not source.is_video:
                            raise RuntimeError("Webcam berhenti memberi frame. Periksa izin/koneksi kamera.")
                        break
                    paused, eof, needs_frame = True, True, False
                    message = "End of video. Tekan 0 atau SPACE untuk replay."
                else:
                    frame = next_frame
                    faces, timings = detector.detect(frame)
                    timings["pose_ms"] = pose_estimator.estimate(frame, faces)
                    observations = analyzer.update(faces, source.timestamp, frame.shape)
                    history.append(timings["inference_ms"])
                    processed += 1
                    processing_ms = (time.perf_counter() - loop_started) * 1000
                    info = {"source": source.label, "index": source.index, "time_s": source.timestamp,
                            "source_fps": source.fps, "processing_fps": 1000/max(processing_ms, 0.001),
                            "p50_ms": float(np.percentile(history, 50)),
                            "p95_ms": float(np.percentile(history, 95)),
                            "confidence": detector.confidence, "iou": detector.iou,
                            "max_yaw": max_yaw, "max_pitch": max_pitch, "max_roll": max_roll,
                            "input_size": detector.size, "threads": args.threads,
                            "owner": analyzer.owner, "calibrated": analyzer.calibrated,
                            "calibration_mode": analyzer.calibration_mode,
                            "auto_calibrate": analyzer.auto_calibrate, **timings}
                    write_record(metrics, {"type": "frame", "segment": segment, **info,
                                          "faces": observations, "events": analyzer.events})
                    for event in analyzer.events:
                        if event.get("event") == "AUTO_CALIBRATED":
                            message = f"Owner = ID {event['id']} (AUTO-CALIBRATED)."
                            if ipc_channel:
                                ipc_channel.send({
                                    "type": "calibration",
                                    "calibrated": True,
                                    "owner_id": event["id"],
                                    "mode": "AUTO",
                                    "message": message,
                                })
                        log(json.dumps(event))
                    if source.is_video and not args.headless and source.frame_count > 1:
                        updating_slider = True
                        cv2.setTrackbarPos("Position", window, round(source.index / (source.frame_count-1)*1000))
                        updating_slider = False

            if ipc_channel:
                while (cmd := ipc_channel.poll_command()) is not None:
                    action = cmd.get("cmd")
                    if action == "calibrate":
                        message = analyzer.calibrate([item["id"] for item in observations])
                        if frame is not None:
                            observations = analyzer.update(faces, source.timestamp, frame.shape)
                        ipc_channel.send({
                            "type": "calibration",
                            "calibrated": analyzer.calibrated,
                            "owner_id": analyzer.owner,
                            "mode": analyzer.calibration_mode,
                            "message": message,
                        })
                        log(message)
                    elif action == "toggle_auto_calibrate":
                        message = analyzer.toggle_auto_calibrate()
                        ipc_channel.send({
                            "type": "calibration",
                            "calibrated": analyzer.calibrated,
                            "owner_id": analyzer.owner,
                            "mode": analyzer.calibration_mode,
                            "message": message,
                        })
                        log(message)
                    elif action == "reset":
                        analyzer.reset()
                        message = "Tracking/timer direset. Duduk di tengah (auto) atau tekan C."
                        if frame is not None:
                            observations = analyzer.update(faces, source.timestamp, frame.shape)
                        log(message)
                    elif action == "pause":
                        paused = True
                        if not source.is_video:
                            analyzer.reset()
                            message = "Pause/resume webcam: kalibrasi ulang agar timer tidak menghitung waktu pause."
                    elif action == "resume":
                        paused = False
                        needs_frame = True
                    elif action == "quit":
                        return

                fps_val = source.fps if source.is_video else (1000.0 / max(processing_ms, 0.001))
                payload = build_state_payload(
                    observations=observations,
                    calibrated=analyzer.calibrated,
                    calibration_mode=analyzer.calibration_mode,
                    auto_calibrate=analyzer.auto_calibrate,
                    owner_id=analyzer.owner,
                    paused=paused,
                    fps=fps_val,
                    mirror=mirror,
                    base_alert_text=getattr(args, "text_alert", "Pengintip Terdeteksi"),
                    base_escalated_text=getattr(args, "text_escalated", "Bahaya! Layar Sedang Diintip"),
                )
                ipc_channel.emit_state_if_needed(payload, heartbeat_s=getattr(args, "ipc_heartbeat_s", 2.0))

            if frame is None:
                break
            if args.headless:
                if args.max_frames and processed >= args.max_frames:
                    break
                continue
            info.update(paused=paused, speed=speed, owner=analyzer.owner, message=message)
            cv2.imshow(window, render(frame, observations, info, mirror))
            if processed == 1:
                try:
                    cv2.setWindowProperty(window, cv2.WND_PROP_TOPMOST, 1)
                    cv2.setWindowProperty(window, cv2.WND_PROP_TOPMOST, 0)
                except cv2.error:
                    pass
            # Semua frame video diproses; jika inference lambat, playback melambat tanpa frame drop.
            wait_ms = max(1, round(1000/source.fps/speed - (time.perf_counter()-loop_started)*1000)) if source.is_video and not paused else 15 if paused else 1
            key = cv2.waitKey(wait_ms) & 0xFF
            if key in (27, ord("q")) or cv2.getWindowProperty(window, cv2.WND_PROP_VISIBLE) < 1:
                break
            if args.max_frames and processed >= args.max_frames:
                break
            needs_frame = not paused
            if key == ord(" "):
                if eof:
                    seek(0)
                    paused = False
                else:
                    paused = not paused
                    if not source.is_video:
                        analyzer.reset()
                        message = "Pause/resume webcam: kalibrasi ulang agar timer tidak menghitung waktu pause."
                needs_frame = not paused
            elif key == ord("n") and source.is_video and not eof:
                paused, needs_frame = True, True
            elif key in (ord("b"), ord("j"), ord("l"), ord("0")) and source.is_video:
                target = {ord("b"): source.index-1, ord("j"): source.index-round(source.fps*5),
                          ord("l"): source.index+round(source.fps*5), ord("0"): 0}[key]
                seek(target)
                if key == ord("b"):
                    paused = True
                elif key == ord("0"):
                    paused = False
                needs_frame = True
            elif key in (ord("+"), ord("="), ord("-")) and source.is_video:
                speed = min(4.0, max(0.25, speed * (0.5 if key == ord("-") else 2)))
            elif key == ord("m"):
                mirror = not mirror
            elif key in (ord("a"), ord("c"), ord("r")):
                if key == ord("r"):
                    analyzer.reset()
                    message = "Tracking/timer direset. Duduk di tengah (auto) atau tekan C."
                    observations = analyzer.update(faces, source.timestamp, frame.shape)
                elif key == ord("a"):
                    message = analyzer.toggle_auto_calibrate()
                else:
                    message = analyzer.calibrate([item["id"] for item in observations])
                    observations = analyzer.update(faces, source.timestamp, frame.shape)
                log(message)
                write_record(metrics, {"type": "control", "action": chr(key), "segment": segment,
                                       "frame": source.index, "message": message, "faces": observations})
        summary = {"type": "summary", "processed_frames": processed,
                   "inference_p50_ms": float(np.percentile(history, 50)) if history else None,
                   "inference_p95_ms": float(np.percentile(history, 95)) if history else None}
        write_record(metrics, summary)
        log(json.dumps(summary))
    except Exception as error:
        if ipc_channel:
            try:
                ipc_channel.send({"type": "error", "fatal": True, "message": str(error)})
            except (OSError, ValueError):
                pass
        raise
    finally:
        if ipc_channel:
            ipc_channel.close()
        pose_estimator.close()
        if source is not None:
            source.close()
        if metrics is not None:
            metrics.close()
        if not args.headless:
            cv2.destroyAllWindows()


def main() -> None:
    """Tanpa input Python; parse CLI dan jalankan eksperimen, tampilkan error tanpa stacktrace panjang."""
    args = arguments()
    is_stdio = getattr(args, "ipc", "none") == "stdio"
    try:
        run(args)
    except KeyboardInterrupt:
        if is_stdio:
            sys.stderr.write("Eksperimen dihentikan.\n")
        else:
            print("Eksperimen dihentikan.")
    except (OSError, RuntimeError, ValueError, cv2.error) as error:
        if is_stdio:
            sys.stderr.write(f"No-Peek: {error}\n")
            sys.exit(1)
        raise SystemExit(f"No-Peek: {error}") from error


if __name__ == "__main__":
    main()
