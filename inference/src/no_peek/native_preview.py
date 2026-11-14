import cv2


class NativePreview:
    def __init__(self):
        self.last_sent = float('-inf')

    def frame(self, image, mirror: bool, now: float) -> dict | None:
        if now - self.last_sent < 0.2:
            return None
        height, width = image.shape[:2]
        scale = min(320 / width, 240 / height, 1.0)
        resized = cv2.resize(image, (max(1, round(width * scale)), max(1, round(height * scale))), interpolation=cv2.INTER_AREA)
        if mirror:
            resized = cv2.flip(resized, 1)
        bgra = cv2.cvtColor(resized, cv2.COLOR_BGR2BGRA)
        self.last_sent = now
        return {"type": "preview_frame", "width": bgra.shape[1], "height": bgra.shape[0], "pixels_hex": bgra.tobytes().hex()}
