use std::io as sio;
use {in_coroutine, offload};
use std;
use std::path::Path;


// Async file
pub struct File {
    inner : Option<std::fs::File>,
}

impl File {
    fn inner_mut(&mut self) -> &std::fs::File {
        self.inner.as_mut().unwrap()
    }

    pub fn open<P: AsRef<Path>>(path: P) -> sio::Result<File> {
        let path = path.as_ref();

        if in_coroutine() {
            offload(|| std::fs::File::open(path))
        } else {
            std::fs::File::open(path)
        }.map(|f| File {
            inner: Some(f)
        })
    }
}

impl sio::Write for File {
    fn write(&mut self, buf : &[u8]) -> sio::Result<usize> {
        if in_coroutine() {
            offload(|| self.inner_mut().write(buf))
        } else {
            self.write(buf)
        }
    }

    fn flush(&mut self) -> sio::Result<()> {
        if in_coroutine() {
            offload(|| self.inner_mut().flush())
        } else {
            self.flush()
        }
    }
}

impl sio::Read for File {

    fn read(&mut self, buf : &mut [u8]) -> sio::Result<usize> {

        if in_coroutine() {
            offload(|| self.inner_mut().read(buf))
        } else {
            self.read(buf)
        }
    }

}
impl Drop for File {
    fn drop(&mut self) {
        if in_coroutine() {
            let inner = self.inner.take().unwrap();
            offload(move || drop(inner) )
        }
    }
}
