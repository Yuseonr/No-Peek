# No-Peek Inference Engine (Python)

Modul Computer Vision lokal untuk deteksi *shoulder surfing* secara *real-time* menggunakan `YOLOv8n-Face` (ONNX Runtime) dan `MediaPipe Face Landmarker` (3D Head Pose).
Berjalan sebagai *worker process* yang berkomunikasi dengan GUI Rust (`../gui`) melalui protokol IPC JSON Lines (NDJSON).
