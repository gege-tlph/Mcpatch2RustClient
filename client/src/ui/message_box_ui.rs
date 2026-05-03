use std::future::Future;
use std::sync::Arc;

use nwd::NwgUi;
use nwg::NativeUi;
use tokio::sync::Mutex;

use crate::ui::OneshotReceiver;

/// 代表一个消息框的UI，目前主要用来显示更新记录，因为有滚动文本框，所以可以显示很多行的文字
/// 
/// 参考链接：https://github.com/gabdube/native-windows-gui/blob/master/native-windows-gui/examples/dialog_multithreading_d.rs
#[derive(NwgUi)]
pub struct MessageBoxWindow {
    #[nwg_control(size: (480, 340), flags: "WINDOW|VISIBLE", center: true, topmost: true)]
    #[nwg_events(OnWindowClose: [MessageBoxWindow::close])]
    window: nwg::Window,

    #[nwg_control(text: "example text", readonly: false)]
    richtext: nwg::TextBox,
}

impl MessageBoxWindow {
    // --- 以下为原有方法 ---
    pub fn popup(title: impl AsRef<str>, content: impl AsRef<str>) -> MessageBoxWindowJoinHandle {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let title = title.as_ref().to_owned();
        let content = content.as_ref().to_owned();

        std::thread::spawn(move || {
            let data = Self {
                window: Default::default(),
                richtext: Default::default(),
            };

            let ui = Self::build_ui(data).unwrap();

            // 从系统 imageres.dll 加载窗口图标
            unsafe {
                use winapi::um::winuser::{LoadImageW, IMAGE_ICON, LR_SHARED, SendMessageW, WM_SETICON};
                use winapi::um::libloaderapi::LoadLibraryW;
                use std::ffi::OsStr;
                use std::os::windows::ffi::OsStrExt;

                let hwnd = ui.window.handle.hwnd().unwrap();
                let wide: Vec<u16> = OsStr::new("imageres.dll").encode_wide().chain(Some(0)).collect();
                let dll = LoadLibraryW(wide.as_ptr());
                if !dll.is_null() {
                    let icon = LoadImageW(
                        dll, 112 as _, IMAGE_ICON, 0, 0, LR_SHARED,
                    );
                    if !icon.is_null() {
                        SendMessageW(hwnd, WM_SETICON, 0, icon as _);
                        SendMessageW(hwnd, WM_SETICON, 1, icon as _);
                    }
                }
            }

            // 调整文本框填充窗口（留边距）
            ui.richtext.set_position(5, 5);
            ui.richtext.set_size(470, 310);

            ui.window.set_text(&title);
            ui.richtext.set_text(&content);

            nwg::dispatch_thread_events();

            // 窗口关闭时，发送一个消息
            tx.send(()).unwrap();
        });

        // 返回一个等待窗口关闭的对象
        MessageBoxWindowJoinHandle::new(rx)
    }

    fn close(&self) {
        nwg::stop_thread_dispatch();
    }
}

/// 用来等待消息窗口关闭的Future对象
pub struct MessageBoxWindowJoinHandle(Arc<Mutex<(Option<OneshotReceiver<()>>, bool)>>);

impl MessageBoxWindowJoinHandle {
    pub fn new(receiver: OneshotReceiver<()>) -> Self {
        Self(Arc::new(Mutex::new((Some(receiver), false))))
    }
}

impl Future for MessageBoxWindowJoinHandle {
    type Output = ();

    fn poll(
        self: std::pin::Pin<&mut Self>, 
        cx: &mut std::task::Context<'_>
    ) -> std::task::Poll<Self::Output> {
        // 首先获取锁
        let lock = self.0.lock();
        tokio::pin!(lock);

        let mut lock = match lock.poll(cx) {
            std::task::Poll::Ready(lock) => lock,
            std::task::Poll::Pending => return std::task::Poll::Pending,
        };

        // 1代表是否已经启动过唤醒线程，为true的话会直接返回，避免重复创建线程
        if lock.1 {
            return std::task::Poll::Ready(());
        }

        // 更新标记
        lock.1 = true;
        
        // 启动唤醒线程
        let this = Arc::clone(&self.0);
        let waker = cx.waker().to_owned();
        std::thread::spawn(move || {
            // 获取锁并拿取receiver对象
            let mut lock = this.blocking_lock();
            let receiver = lock.0.take().unwrap();

            // 等待窗口被关闭的消息发送过来
            receiver.blocking_recv().unwrap();
            
            // 窗口被关闭时，唤醒waker
            waker.wake();
        });

        std::task::Poll::Pending
    }
}
