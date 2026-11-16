//! A little-endian cursor that errors instead of reading past the end.

pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

const TRUNCATED: &str = "the file ends early";

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    pub fn seek(&mut self, at: usize) -> Result<(), String> {
        if at > self.data.len() {
            return Err(TRUNCATED.into());
        }
        self.at = at;
        Ok(())
    }

    pub fn skip(&mut self, n: usize) -> Result<(), String> {
        self.bytes(n).map(|_| ())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or(TRUNCATED)?;
        let out = self.data.get(self.at..end).ok_or(TRUNCATED)?;
        self.at = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.bytes(N)?.try_into().map_err(|_| TRUNCATED.to_string())
    }

    pub fn i8(&mut self) -> Result<i8, String> {
        Ok(i8::from_le_bytes(self.array()?))
    }

    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn vec3(&mut self) -> Result<[f32; 3], String> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// A 1-based fragment reference as a 0-based index; `None` for 0 or a name reference.
    pub fn frag(&mut self) -> Result<Option<usize>, String> {
        let v = self.i32()?;
        Ok(usize::try_from(v).ok().and_then(|v| v.checked_sub(1)))
    }
}
