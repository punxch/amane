//! 桌面外壳: 启动内嵌服务, 分配端口与令牌, 用同一份界面加载.
//!
//! 关闭窗口即退出进程; 登记与任务都已持久化, 下次启动按基准时刻继续.

use amane_core::config::{ColdSettings, HotSettings, Settings};
use rand::RngCore;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(async move {
                // 令牌只存在于本次进程: 未携带的本地请求会被拒绝.
                let mut bytes = [0_u8; 32];
                rand::rng().fill_bytes(&mut bytes);
                let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

                // 数据目录独立, 两个实现可同时安装.
                let data_dir = handle.path().app_data_dir().expect("应取得应用数据目录");

                let settings = Settings {
                    cold: ColdSettings {
                        data_dir,
                        // 端口由系统分配, 不会与已运行的实例冲突.
                        bind: "127.0.0.1:0".to_owned(),
                        api_token: Some(token.clone()),
                    },
                    hot: HotSettings::default(),
                };

                let state = amane_server::boot(settings).await.expect("内嵌服务应启动");
                let (address, _server) = amane_server::start(state).await.expect("服务应绑定端口");

                let url = format!("http://{address}/?token={token}");
                tauri::WebviewWindowBuilder::new(
                    &handle,
                    "main",
                    tauri::WebviewUrl::External(url.parse().expect("地址应合法")),
                )
                .title("Amane")
                .inner_size(1280.0, 820.0)
                .build()
                .expect("应创建主窗口");
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("桌面外壳应启动");
}
