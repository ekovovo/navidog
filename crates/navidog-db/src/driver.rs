//! 驱动抽象：`Driver` 创建 `Session`，会话按数据库类别暴露 `Relational`、`Document`、`KeyValue`。
//! 一期只实现 `Relational`；另外两类只定义接口，M14 接入 MongoDB、Redis 时实现。

use std::sync::Arc;

use futures::future::BoxFuture;
use gpui_kit::{App, Global, SharedString};
use navidog_core::registry::Registry;

use crate::params::ConnectParams;

/// 驱动错误。`code` 保留数据库的原始错误码，显示为 Navicat 的“1045 - Access denied …”格式。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct DbError {
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{code} - {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl DbError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self {
            code: None,
            message: message.into(),
        }
    }

    pub fn coded(code: impl ToString, message: impl Into<String>) -> Self {
        Self {
            code: Some(code.to_string()),
            message: message.into(),
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Relational,
    Document,
    KeyValue,
}

#[derive(Clone, Debug)]
pub struct DriverInfo {
    /// 注册键，也是 connections.json 中的 `driver`
    pub id: SharedString,
    pub name: SharedString,
    pub family: Family,
    pub default_port: Option<u16>,
    pub supports_ssl: bool,
    pub supports_ssh: bool,
}

pub trait Driver: Send + Sync + 'static {
    fn info(&self) -> &DriverInfo;
    fn connect(&self, params: ConnectParams) -> BoxFuture<'static, DbResult<Box<dyn Session>>>;
}

/// 取消正在执行的语句。
pub type CancelHandle = Arc<dyn Fn() -> BoxFuture<'static, DbResult<()>> + Send + Sync>;

pub trait Session: Send + 'static {
    /// 连接时取得的服务器版本
    fn server_version(&self) -> &str;
    fn ping(&mut self) -> BoxFuture<'_, DbResult<()>>;
    fn cancel_handle(&self) -> Option<CancelHandle>;
    fn close(self: Box<Self>) -> BoxFuture<'static, DbResult<()>>;

    fn relational(&mut self) -> Option<&mut dyn Relational> {
        None
    }
    fn document(&mut self) -> Option<&mut dyn Document> {
        None
    }
    fn key_value(&mut self) -> Option<&mut dyn KeyValue> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// 类型名；文本协议拿不到类型时为空
    pub type_name: String,
}

/// 文本形式的结果集，`None` 表示 NULL。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Option<String>>>,
    pub affected_rows: Option<u64>,
}

pub trait Relational: Send {
    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>>;
    /// 以文本协议执行一条或多条语句，每条返回一个结果集。
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, DbResult<Vec<ResultSet>>>;
}

pub trait Document: Send {
    fn list_databases(&mut self) -> BoxFuture<'_, DbResult<Vec<String>>>;
    fn list_collections<'a>(
        &'a mut self,
        database: &'a str,
    ) -> BoxFuture<'a, DbResult<Vec<String>>>;
    /// 按 JSON 筛选条件查询，返回 JSON 文本形式的文档。
    fn find<'a>(
        &'a mut self,
        database: &'a str,
        collection: &'a str,
        filter_json: &'a str,
        limit: u64,
    ) -> BoxFuture<'a, DbResult<Vec<String>>>;
}

pub trait KeyValue: Send {
    fn databases(&mut self) -> BoxFuture<'_, DbResult<Vec<u32>>>;
    /// SCAN 一批键，返回下一个游标与键列表。
    fn scan<'a>(
        &'a mut self,
        db: u32,
        pattern: &'a str,
        cursor: u64,
        count: u32,
    ) -> BoxFuture<'a, DbResult<(u64, Vec<String>)>>;
    fn get<'a>(&'a mut self, db: u32, key: &'a str) -> BoxFuture<'a, DbResult<Option<String>>>;
}

/// 已注册的驱动：内置驱动与插件 JS 驱动（M5）共用。
#[derive(Default)]
pub struct DriverRegistry(pub Registry<Arc<dyn Driver>>);

impl Global for DriverRegistry {}

impl DriverRegistry {
    pub fn get(cx: &App, id: &str) -> Option<Arc<dyn Driver>> {
        cx.global::<Self>().0.get(id).cloned()
    }

    pub fn all(cx: &App) -> Vec<Arc<dyn Driver>> {
        cx.global::<Self>()
            .0
            .iter()
            .map(|(_, _, d)| d.clone())
            .collect()
    }
}
