//! 驱动依赖 tokio，而 GPUI 的执行器不是 tokio 运行时，所以驱动 I/O 统一跑在这里。
//! 返回的 `JoinHandle` 可以在 GPUI 的 `cx.spawn` 中直接 await。

use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("navidog-db")
        .enable_all()
        .build()
        .expect("无法启动数据库运行时")
});

pub fn runtime() -> &'static Runtime {
    &RUNTIME
}

pub fn spawn<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> JoinHandle<T> {
    RUNTIME.spawn(fut)
}
