#[derive(Clone, Debug, PartialEq)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl PreviewFrame {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 320
            || self.height > 240
            || self.bgra.len() != self.width as usize * self.height as usize * 4
            || self
                .bgra
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 255)
        {
            return Err("Invalid opaque BGRA preview buffer".into());
        }
        Ok(())
    }
    pub fn decode(width: u32, height: u32, encoded: &str) -> Result<Self, String> {
        if width == 0 || height == 0 || width > 320 || height > 240 {
            return Err("Preview dimensions must be within 320x240".into());
        }
        let length = width as usize * height as usize * 4;
        if encoded.len() != length * 2 {
            return Err("Preview byte count does not match dimensions".into());
        }
        fn nibble(byte: u8) -> Result<u8, String> {
            match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                _ => Err("Invalid preview hex encoding".into()),
            }
        }
        let bgra = encoded
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| Ok(nibble(pair[0])? * 16 + nibble(pair[1])?))
            .collect::<Result<Vec<_>, String>>()?;
        if bgra.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
            return Err("Preview pixels must be opaque".into());
        }
        Ok(Self {
            width,
            height,
            bgra,
        })
    }
}

#[derive(Default)]
pub struct PreviewMailbox(std::sync::Mutex<Option<PreviewFrame>>);

impl PreviewMailbox {
    pub fn put(&self, frame: PreviewFrame) -> Result<bool, String> {
        let mut slot = self
            .0
            .lock()
            .map_err(|error| format!("Preview mailbox poisoned: {error}"))?;
        let notify = slot.is_none();
        *slot = Some(frame);
        Ok(notify)
    }

    pub fn take(&self) -> Result<Option<PreviewFrame>, String> {
        Ok(self
            .0
            .lock()
            .map_err(|error| format!("Preview mailbox poisoned: {error}"))?
            .take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_pixels_and_mailbox_keeps_only_latest_frame() {
        let first = PreviewFrame::decode(1, 1, "ff0000ff").unwrap();
        assert_eq!(first.bgra, vec![255, 0, 0, 255]);
        for (w, h, data) in [
            (0, 1, ""),
            (321, 1, ""),
            (1, 1, "00"),
            (1, 1, "gg0000ff"),
            (1, 1, "00000000"),
        ] {
            assert!(PreviewFrame::decode(w, h, data).is_err());
        }
        let mailbox = PreviewMailbox::default();
        assert!(mailbox.put(first).unwrap());
        let last = PreviewFrame::decode(1, 1, "0000ffff").unwrap();
        assert!(!mailbox.put(last.clone()).unwrap());
        assert_eq!(mailbox.take().unwrap(), Some(last));
        assert!(mailbox.take().unwrap().is_none());
    }
}
