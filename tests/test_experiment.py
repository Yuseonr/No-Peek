"""Uji matematika decoder dan temporal tanpa kamera, tanpa bobot tambahan."""

import unittest

import numpy as np

from no_peek.analysis import Analyzer, orientation
from no_peek.detector import Face, decode_maps, letterbox, suppress_overlaps
from no_peek.geometry import estimate_distance_cm, is_side_by_side_friend
from no_peek.pose import crop_face, euler_to_matrix, matrix_to_angles
from no_peek.preview import render


def sample_face(x: float = 100, y: float = 100, size: float = 100) -> Face:
    """Input posisi (x,y) dan ukuran; output wajah frontal sintetis dengan lima landmark konsisten."""
    return Face(
        np.array([x, y, x + size, y + size], dtype=float),
        0.9,
        np.array([
            [x + 0.25 * size, y + 0.30 * size],
            [x + 0.75 * size, y + 0.30 * size],
            [x + 0.50 * size, y + 0.55 * size],
            [x + 0.30 * size, y + 0.75 * size],
            [x + 0.70 * size, y + 0.75 * size],
        ], dtype=float),
    )


class DecoderTests(unittest.TestCase):
    """Pastikan transformasi input, DFL, confidence, dan NMS punya hasil yang diketahui."""

    def test_letterbox_and_coordinate_inverse(self):
        """Frame non-square -> tensor square; inverse titik harus kembali ke koordinat asli."""
        tensor, scale, padding = letterbox(np.zeros((720, 1280, 3), np.uint8), (640, 640))
        self.assertEqual(tensor.shape, (1, 3, 640, 640))
        self.assertEqual(padding, (0, 140))
        point = np.array([640., 360.])
        np.testing.assert_allclose(((point*scale+padding)-padding)/scale, point)

    def test_raw_map_decoding(self):
        """Satu anchor skor tinggi dan DFL bin 1 -> box/landmark hasil hitungan manual."""
        outputs = [np.full((1, 80, size, size), -80., np.float32) for size in (80, 40, 20)]
        feature = outputs[0]
        feature[0, :64, 10, 20] = -80
        for side in range(4):
            feature[0, side*16+1, 10, 20] = 80
        feature[0, 64, 10, 20] = 80
        feature[0, 65:, 10, 20] = 0
        rows = decode_maps(outputs, .5)
        self.assertEqual(rows.shape, (1, 15))
        np.testing.assert_allclose(rows[0, :4], [156, 76, 172, 92])
        np.testing.assert_allclose(rows[0, 5:].reshape(5, 2), [[160, 80]]*5)

    def test_nms_and_no_detections(self):
        """Dua box identik harus menjadi satu; tensor confidence rendah menghasilkan kosong."""
        self.assertEqual(suppress_overlaps(np.array([[0, 0, 10, 10], [0, 0, 10, 10]]), np.array([.9, .8]), .45), [0])
        rows = decode_maps([np.full((1, 80, n, n), -80.) for n in (80, 40, 20)], .5)
        self.assertEqual(rows.shape, (0, 15))


class TemporalTests(unittest.TestCase):
    """Uji alert berdasarkan detik sumber, tracking owner, grace period, dan reset seek."""

    def test_alert_once_per_transition(self):
        """Observer frontal -> pending, alert 2s, eskalasi 7s; episode tidak spam."""
        analyzer = Analyzer()
        owner = sample_face()
        first = analyzer.update([owner], 0., (720, 1280, 3))
        analyzer.calibrate([first[0]['id']])
        events = []
        for tick in range(81):
            rows = analyzer.update([owner, sample_face(400)], tick/10, (720, 1280, 3))
            events.extend(analyzer.events)
            if tick == 19:
                self.assertEqual(rows[1]['state'], 'PENDING')
            if tick == 20:
                self.assertEqual(rows[1]['state'], 'ALERT')
        self.assertEqual([e['event'] for e in events], ['ALERT', 'ESCALATED'])

    def test_grace_and_owner_loss(self):
        """Miss 100ms mempertahankan ID; miss >250ms mencabut kalibrasi, mencegah owner swap."""
        analyzer = Analyzer()
        first = analyzer.update([sample_face()], 0., (720, 1280, 3))
        analyzer.calibrate([first[0]['id']])
        analyzer.update([], .1, (720, 1280, 3))
        self.assertEqual(analyzer.update([sample_face()], .2, (720, 1280, 3))[0]['role'], 'OWNER')
        analyzer.update([], .6, (720, 1280, 3))
        self.assertFalse(analyzer.calibrated)
        self.assertEqual(analyzer.update([sample_face()], .7, (720, 1280, 3))[0]['role'], 'UNCALIBRATED')

    def test_orientation_reset_and_calibration_guard(self):
        """Proxy menoleh memutus timer; reset seek menghapus owner dan seluruh track."""
        analyzer = Analyzer()
        self.assertIn('gagal', analyzer.calibrate([1, 2]))
        face = sample_face()
        self.assertTrue(orientation(face.landmarks)['frontal_proxy'])
        face.landmarks[2, 0] += 40
        self.assertFalse(orientation(face.landmarks)['frontal_proxy'])
        analyzer.update([face], 0., (720, 1280, 3))
        analyzer.reset()
        self.assertEqual(analyzer.tracks, {})
        self.assertIsNone(analyzer.owner)

    def test_risk_grace_and_turned_face_reset(self):
        """Miss pendek menjaga episode; observer menoleh mereset durasi tanpa alert."""
        analyzer = Analyzer()
        owner, observer = sample_face(), sample_face(400)
        analyzer.update([owner], 0., (720, 1280, 3))
        analyzer.calibrate([1])
        for tick in range(16):
            analyzer.update([owner, observer], tick/10, (720, 1280, 3))
        analyzer.update([owner], 1.6, (720, 1280, 3))
        rows = analyzer.update([owner, observer], 1.7, (720, 1280, 3))
        self.assertAlmostEqual(rows[1]['risk_s'], 1.7)
        observer.landmarks[2, 0] += 40
        rows = analyzer.update([owner, observer], 1.8, (720, 1280, 3))
        self.assertEqual(rows[1]['state'], 'MONITORING')
        self.assertEqual(rows[1]['risk_s'], 0.)
        self.assertEqual(analyzer.events, [])


class HeadPoseTests(unittest.TestCase):
    """Uji dekomposisi matriks transformasi 3D MediaPipe, crop persegi, dan filter BR-05 yaw/pitch."""

    def test_matrix_to_angles_roundtrip(self):
        """Matriks rotasi 3D Ry(yaw)*Rx(pitch)*Rz(roll) harus kembali ke sudut derajat semula."""
        for yaw, pitch, roll in ((0., 0., 0.), (22.5, -14., 8.), (-38., 28., -15.)):
            angles = matrix_to_angles(euler_to_matrix(yaw, pitch, roll))
            self.assertIsNotNone(angles)
            self.assertAlmostEqual(angles['yaw_deg'], yaw, places=4)
            self.assertAlmostEqual(angles['pitch_deg'], pitch, places=4)
            self.assertAlmostEqual(angles['roll_deg'], roll, places=4)
        self.assertIsNone(matrix_to_angles(np.zeros((4, 4))))

    def test_crop_face_square_and_border_padding(self):
        """Crop wajah di tepi frame harus tetap persegi 1:1 dan RGB contiguous."""
        frame = np.zeros((240, 320, 3), dtype=np.uint8)
        crop = crop_face(frame, np.array([0., 0., 60., 100.]), margin=0.25)
        self.assertEqual(crop.shape[0], crop.shape[1])
        self.assertEqual(crop.shape[2], 3)
        self.assertTrue(crop.flags['C_CONTIGUOUS'])

    def test_mediapipe_pose_overrides_proxy_for_pitch_and_yaw(self):
        """Observer menunduk (|pitch|>=25) atau menoleh (|yaw|>=35) memutus risiko meski 5 titik simetris."""
        analyzer = Analyzer(max_yaw=35., max_pitch=25., max_roll=35.)
        owner = sample_face(100)
        owner.pose = matrix_to_angles(euler_to_matrix(0., 0., 0.))
        analyzer.update([owner], 0., (720, 1280, 3))
        analyzer.calibrate([1])

        observer = sample_face(400)
        observer.pose = matrix_to_angles(euler_to_matrix(10., -10., 0.))
        rows = analyzer.update([owner, observer], 0.1, (720, 1280, 3))
        self.assertEqual(rows[1]['pose_source'], 'mediapipe')
        self.assertEqual(rows[1]['state'], 'PENDING')

        # Menunduk melihat HP (pitch = -30 deg) -> melebihi ambang 25 deg, timer wajib reset.
        observer.pose = matrix_to_angles(euler_to_matrix(5., -30., 0.))
        rows = analyzer.update([owner, observer], 0.2, (720, 1280, 3))
        self.assertFalse(rows[1]['frontal_proxy'])
        self.assertEqual(rows[1]['state'], 'MONITORING')
        self.assertEqual(rows[1]['risk_s'], 0.)

        # Menoleh ke samping (yaw = 40 deg) -> melebihi ambang 35 deg, tetap MONITORING.
        observer.pose = matrix_to_angles(euler_to_matrix(40., 0., 0.))
        rows = analyzer.update([owner, observer], 0.3, (720, 1280, 3))
        self.assertFalse(rows[1]['frontal_proxy'])
        self.assertEqual(rows[1]['state'], 'MONITORING')
        info = dict(source='test', paused=False, speed=1., index=3, time_s=.3,
                    source_fps=30., processing_fps=30., inference_ms=2., detector_ms=4.,
                    pose_ms=3., p50_ms=2., p95_ms=2., confidence=.5, iou=.45,
                    max_yaw=35., max_pitch=25., input_size=(640, 640), threads=2,
                    owner=1, message='ok')
        canvas = render(np.zeros((720, 1280, 3), np.uint8), rows, info, mirror=True)
        self.assertEqual(canvas.shape, (850, 1400, 3))


class AutoCalibrationAndFriendTests(unittest.TestCase):
    """Uji estimasi jarak, auto-kalibrasi owner di tengah, dan klasifikasi FRIEND berdampingan."""

    def test_distance_estimation_and_yaw_compensation(self):
        """Wajah lebih besar -> jarak lebih dekat; kompensasi yaw menjaga jarak saat menoleh."""
        near = sample_face(500, 200, size=240)
        far = sample_face(500, 200, size=100)
        dist_near = estimate_distance_cm(near.box, near.landmarks, 1280)
        dist_far = estimate_distance_cm(far.box, far.landmarks, 1280)
        self.assertLess(dist_near, 80.0)
        self.assertGreater(dist_far, 130.0)

        # Saat menoleh 30 deg, jarak antar mata 2D menyempit cos(30); kompensasi yaw mencegah jarak membengkak.
        turned_landmarks = near.landmarks.copy()
        eye_center = turned_landmarks[:2].mean(axis=0)
        turned_landmarks[:2] = eye_center + (turned_landmarks[:2] - eye_center) * np.cos(np.radians(30.0))
        dist_compensated = estimate_distance_cm(near.box, turned_landmarks, 1280, yaw_deg=30.0)
        self.assertAlmostEqual(dist_near, dist_compensated, delta=1.0)

    def test_auto_calibration_locks_center_closest_face(self):
        """Wajah dekat di zona tengah otomatis menjadi OWNER setelah stabil >= 0.6s."""
        analyzer = Analyzer(auto_calibrate=True, auto_calibrate_s=0.6)
        center_owner = sample_face(520, 220, size=240)  # cx = 640 (tengah), dist ~65 cm
        back_peeker = sample_face(900, 120, size=95)    # cx = 947.5, dist ~164 cm
        for tick in range(6):
            rows = analyzer.update([center_owner, back_peeker], tick * 0.1, (720, 1280, 3))
            self.assertFalse(analyzer.calibrated)
            self.assertEqual(rows[0]['role'], 'UNCALIBRATED')

        rows = analyzer.update([center_owner, back_peeker], 0.6, (720, 1280, 3))
        self.assertTrue(analyzer.calibrated)
        self.assertEqual(analyzer.calibration_mode, 'AUTO')
        self.assertEqual(rows[0]['role'], 'OWNER')
        self.assertEqual(rows[1]['role'], 'OBSERVER')
        self.assertTrue(any(e['event'] == 'AUTO_CALIBRATED' for e in analyzer.events))

    def test_side_by_side_friend_including_smaller_head_vs_observer(self):
        """Teman duduk berdampingan (termasuk kepala lebih kecil) = FRIEND; orang jauh/di atas = OBSERVER."""
        analyzer = Analyzer(auto_calibrate=True, auto_calibrate_s=0.0)
        owner = sample_face(520, 240, size=240)              # cx=640, cy=360, dist ~65 cm
        friend_equal = sample_face(180, 250, size=220)       # sebelah kiri, sejajar, dist ~71 cm
        friend_small_head = sample_face(840, 270, size=155)  # sebelah kanan, sejajar, kepala mungil (~100 cm)
        peeker_behind = sample_face(900, 80, size=95)        # tinggi di belakang (>150 cm)

        for tick in range(25):
            rows = analyzer.update(
                [owner, friend_equal, friend_small_head, peeker_behind],
                tick * 0.1,
                (720, 1280, 3),
            )
        roles = {row['id']: row['role'] for row in rows}
        states = {row['id']: row['state'] for row in rows}
        self.assertEqual(roles[1], 'OWNER')
        self.assertEqual(roles[2], 'FRIEND')
        self.assertEqual(roles[3], 'FRIEND')
        self.assertEqual(roles[4], 'OBSERVER')
        # FRIEND tidak boleh memicu PENDING/ALERT walaupun menghadap layar > 2 detik.
        self.assertEqual(states[2], 'MONITORING')
        self.assertEqual(states[3], 'MONITORING')
        # OBSERVER di belakang tetap memicu ALERT setelah >= 2 detik.
        self.assertEqual(states[4], 'ALERT')


if __name__ == '__main__':
    unittest.main()
