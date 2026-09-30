use super::SshSpec;

pub struct Tunnel {
    pub local_port: u16,
}

impl Tunnel {
    pub async fn open(_ssh: &SshSpec, _remote_host: &str, _remote_port: u16) -> anyhow::Result<Tunnel> {
        todo!()
    }
}
