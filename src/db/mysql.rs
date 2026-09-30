use tokio::sync::mpsc;

use super::*;
use crate::conn::ConnSpec;

pub struct MyConn {
    info: ServerInfo,
}

#[derive(Clone)]
pub struct MyCancel {}

impl MyCancel {
    pub async fn cancel(&self) -> DbResult<()> {
        todo!()
    }
}

impl MyConn {
    pub async fn connect(_spec: &ConnSpec) -> DbResult<Self> {
        todo!()
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    pub async fn execute(&mut self, _sql: &str, _tx: &mpsc::Sender<ExecEvent>) -> DbResult<()> {
        todo!()
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        todo!()
    }

    pub fn in_transaction(&self) -> bool {
        todo!()
    }

    pub async fn change_database(&mut self, _database: &str) -> DbResult<()> {
        todo!()
    }

    pub async fn load_catalog(&mut self) -> DbResult<Catalog> {
        todo!()
    }

    pub async fn list_databases(&mut self) -> DbResult<Vec<String>> {
        todo!()
    }

    pub async fn list_relations(&mut self, _schema: &str) -> DbResult<Vec<Relation>> {
        todo!()
    }

    pub async fn table_details(&mut self, _schema: Option<&str>, _table: &str) -> DbResult<TableDetails> {
        todo!()
    }

    pub async fn object_ddl(&mut self, _schema: Option<&str>, _name: &str, _kind: &str) -> DbResult<String> {
        todo!()
    }

    pub async fn activity(&mut self) -> DbResult<ResultSet> {
        todo!()
    }

    pub async fn kill_session(&mut self, _id: &str) -> DbResult<()> {
        todo!()
    }

    pub async fn explain(&mut self, _sql: &str, _analyze: bool) -> DbResult<PlanNode> {
        todo!()
    }
}
