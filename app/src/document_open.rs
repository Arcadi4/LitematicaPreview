use std::ffi::CStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};

use crate::app::App;

const CORE: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = rune::AppHandle<App>]
    struct DocumentReceiver;

    impl DocumentReceiver {
        #[unsafe(method(openDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let Some(list) = event.paramDescriptorForKeyword(DIRECT_OBJECT) else { return };
            let paths: Vec<_> = (1..=list.numberOfItems()).filter_map(|index| {
                let url = list.descriptorAtIndex(index)?.fileURLValue()?;
                // The file-system representation borrows the URL and is NUL-terminated.
                let ptr = url.fileSystemRepresentation();
                let bytes = unsafe { CStr::from_ptr(ptr.as_ptr()) }.to_bytes();
                Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
            }).collect();
            if !paths.is_empty() {
                let app = self.ivars();
                app.update(move |s, _| { for path in paths { s.queue_open(path); } });
                app.focus();
            }
        }
    }
);

pub(crate) struct OpenDocuments {
    manager: Retained<NSAppleEventManager>,
    _receiver: Retained<DocumentReceiver>,
}

impl OpenDocuments {
    pub(crate) fn install(app: rune::AppHandle<App>) -> Option<Self> {
        let main = MainThreadMarker::new()?;
        let receiver: Retained<DocumentReceiver> =
            unsafe { msg_send![super(DocumentReceiver::alloc(main).set_ivars(app)), init] };
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // AppKit invokes the registered selector on the main thread. The guard retains its target.
        unsafe {
            manager.setEventHandler_andSelector_forEventClass_andEventID(
                &receiver,
                sel!(openDocuments:withReplyEvent:),
                CORE,
                OPEN_DOCUMENTS,
            );
        }
        Some(Self {
            manager,
            _receiver: receiver,
        })
    }
}

impl Drop for OpenDocuments {
    fn drop(&mut self) {
        self.manager
            .removeEventHandlerForEventClass_andEventID(CORE, OPEN_DOCUMENTS);
    }
}
