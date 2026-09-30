use std::io;

#[derive(Default)]
pub struct Sinks {}

impl Sinks {
    pub fn tee(&mut self, _path: &str, _overwrite: bool) -> io::Result<()> {
        todo!()
    }
    pub fn notee(&mut self) {
        todo!()
    }
    pub fn once(&mut self, _path: &str, _overwrite: bool) -> io::Result<()> {
        todo!()
    }
    pub fn pipe_once(&mut self, _command: &str) -> io::Result<()> {
        todo!()
    }
    pub fn write_result(&mut self, _plain_text: &str) -> io::Result<()> {
        todo!()
    }
    pub fn is_active(&self) -> bool {
        todo!()
    }
    pub fn is_redirected_once(&self) -> bool {
        todo!()
    }
}
