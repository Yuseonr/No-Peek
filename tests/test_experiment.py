"""Uji matematika decoder dan temporal tanpa kamera, tanpa bobot tambahan."""

import unittest

import numpy as np

from no_peek.analysis import Analyzer, orientation
from no_peek.detector import Face, decode_maps, letterbox, suppress_overlaps


def sample_face(x: float = 100) -> Face:
    """Input posisi x; output wajah frontal sintetis dengan lima landmark konsisten."""
    return Face(np.array([x, 100, x+100, 200]), .9,
                np.array([[x+25, 130], [x+75, 130], [x+50, 155], [x+30, 175], [x+70, 175]]))


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


if __name__ == '__main__':
    unittest.main()
