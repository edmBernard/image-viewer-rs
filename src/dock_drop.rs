use std::sync::Mutex;

use bevy::prelude::*;
use bevy::winit::EventLoopProxy;
use bevy::winit::EventLoopProxyWrapper;
use bevy::winit::WinitUserEvent;

use crate::{AddMode, LoadImagesEvent};

// MARK: macOS Dock Drop
// macOS requires the NSApplicationDelegate to implement application:openURLs: for dock icon
// drops and "Open With" to work. Winit 0.30.x doesn't implement this, so we inject the method
// into WinitApplicationDelegate at runtime. When winit 0.31+ lands (which exposes delegate
// registration), this workaround can be removed.

static DOCK_DROP_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

// The app only updates on input, so the ObjC handler wakes the event loop itself after queueing paths
static EVENT_LOOP_PROXY: Mutex<Option<EventLoopProxy<WinitUserEvent>>> = Mutex::new(None);

pub fn store_event_loop_proxy(proxy: Res<EventLoopProxyWrapper>) {
    let Ok(mut stored) = EVENT_LOOP_PROXY.lock() else {
        return;
    };
    *stored = Some((**proxy).clone());
}

#[cfg(target_os = "macos")]
pub mod macos_dock_drop {
    use objc2::ffi;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::sel;
    use objc2_foundation::{NSArray, NSURL};

    use bevy::winit::WinitUserEvent;

    // The handler that the ObjC runtime will call for application:openURLs:
    // Signature: void(id self, SEL _cmd, id application, id urls)
    unsafe extern "C" fn handle_open_urls(_this: &AnyObject, _cmd: Sel, _sender: &AnyObject, urls: &NSArray<NSURL>) {
        let mut paths = Vec::new();
        for url in urls.iter() {
            let Some(ns_path) = url.path() else {
                continue;
            };
            let path = ns_path.to_string();
            println!("macOS dock drop: {}", path);
            paths.push(path);
        }

        if !paths.is_empty() {
            let Ok(mut queue) = super::DOCK_DROP_QUEUE.lock() else {
                return;
            };
            queue.extend(paths);
            drop(queue);
            wake_event_loop();
        }
    }

    fn wake_event_loop() {
        let Ok(stored) = super::EVENT_LOOP_PROXY.lock() else {
            return;
        };
        let Some(proxy) = stored.as_ref() else {
            return;
        };
        let _ = proxy.send_event(WinitUserEvent::WakeUp);
    }

    // Inject application:openURLs: into WinitApplicationDelegate at runtime.
    // Must be called after the event loop is created (so the class is registered).
    pub fn inject_open_urls_handler() {
        let Some(cls) = AnyClass::get(c"WinitApplicationDelegate") else {
            println!("macOS dock drop: WinitApplicationDelegate class not found, skipping");
            return;
        };

        let sel = sel!(application:openURLs:);
        // Type encoding: void(id, SEL, id, id) = "v@:@@"
        let types = c"v@:@@";
        let imp: Imp = unsafe { std::mem::transmute(handle_open_urls as *const ()) };
        let cls_ptr = cls as *const AnyClass as *mut AnyClass;

        // SAFETY: cls_ptr points to a valid, registered ObjC class. The selector, type encoding,
        // and function signature all match the application:openURLs: delegate method.
        let ok = unsafe { ffi::class_addMethod(cls_ptr, sel, imp, types.as_ptr()) };
        if ok.as_bool() {
            println!("macOS dock drop: injected application:openURLs: handler");
        } else {
            println!("macOS dock drop: failed to inject handler (method may already exist)");
        }
    }
}

pub fn poll_dock_drop_queue(mut load_image_evw: MessageWriter<LoadImagesEvent>, add_mode: Res<AddMode>) {
    let Ok(mut queue) = DOCK_DROP_QUEUE.lock() else {
        return;
    };
    if queue.is_empty() {
        return;
    }
    load_image_evw.write(LoadImagesEvent {
        paths: queue.drain(..).collect(),
        replace: !add_mode.0,
    });
}
