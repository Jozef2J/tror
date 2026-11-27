use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub struct Promise {
    stage: Mutex<Stage>,
    signal: Condvar,
}

enum Stage {
    Waiting,
    Ready(Vec<PathBuf>),
    Failed,
}

impl Promise {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            stage: Mutex::new(Stage::Waiting),
            signal: Condvar::new(),
        })
    }

    pub fn fulfil(&self, tops: Vec<PathBuf>) {
        *self.stage.lock().unwrap_or_else(|e| e.into_inner()) = Stage::Ready(tops);
        self.signal.notify_all();
    }

    pub fn fail(&self) {
        let mut stage = self.stage.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*stage, Stage::Waiting) {
            *stage = Stage::Failed;
        }
        self.signal.notify_all();
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    fn ready(&self) -> Option<Vec<PathBuf>> {
        match &*self.stage.lock().unwrap_or_else(|e| e.into_inner()) {
            Stage::Ready(tops) => Some(tops.clone()),
            _ => None,
        }
    }

    #[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
    fn wait(&self) -> Option<Vec<PathBuf>> {
        let until = Instant::now() + Duration::from_secs(30 * 60);
        let mut stage = self.stage.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            match &*stage {
                Stage::Ready(tops) => return Some(tops.clone()),
                Stage::Failed => return None,
                Stage::Waiting => {}
            }
            let left = until.checked_duration_since(Instant::now())?;
            stage = self
                .signal
                .wait_timeout(stage, left.min(Duration::from_secs(1)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct Entry {
    name: String,
    path: PathBuf,
    dir: bool,
    size: u64,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn entries(tops: &[PathBuf]) -> Vec<Entry> {
    fn walk(path: &Path, name: String, out: &mut Vec<Entry>) {
        let Ok(meta) = std::fs::metadata(path) else {
            return;
        };
        let dir = meta.is_dir();
        out.push(Entry {
            name: name.clone(),
            path: path.to_path_buf(),
            dir,
            size: if dir { 0 } else { meta.len() },
        });
        if dir && let Ok(read) = std::fs::read_dir(path) {
            let mut children: Vec<_> = read.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                let leaf = child
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                walk(&child, format!("{name}\\{leaf}"), out);
            }
        }
    }
    let mut out = Vec::new();
    for top in tops {
        let name = top
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        walk(top, name, &mut out);
    }
    out
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn place(source: &Path, target: &Path) -> std::io::Result<()> {
    if std::fs::rename(source, target).is_ok() {
        return Ok(());
    }
    if source.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            place(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, target).map(|_| ())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn start(
    _frame: &eframe::Frame,
    _items: Vec<(String, bool)>,
    promise: Arc<Promise>,
    _icon: &[u8],
    _done: impl Fn(bool) + Send + 'static,
) -> Result<(), String> {
    promise.fail();
    Err("dragging files out is not supported here".to_owned())
}

#[cfg(target_os = "windows")]
use windows::{
    Win32::{
        Foundation::{
            BOOL, COLORREF, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
            DV_E_FORMATETC, E_FAIL, E_NOTIMPL, HGLOBAL, OLE_E_ADVISENOTSUPPORTED, POINT, S_OK,
            SIZE,
        },
        Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateDIBSection, DIB_RGB_COLORS},
        Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
        System::{
            Com::{
                CLSCTX_ALL, CoCreateInstance, DATADIR_GET, DVASPECT_CONTENT, FORMATETC,
                IAdviseSink, IBindCtx, IDataObject, IDataObject_Impl, IEnumFORMATETC,
                IEnumSTATDATA, STGM_READ, STGM_SHARE_DENY_WRITE, STGMEDIUM, STGMEDIUM_0,
                TYMED_HGLOBAL, TYMED_ISTREAM,
            },
            DataExchange::RegisterClipboardFormatW,
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::{
                DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl,
                OleInitialize,
            },
            SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        },
        UI::Shell::{
            CFSTR_FILECONTENTS, CFSTR_FILEDESCRIPTORW, CLSID_DragDropHelper, FD_ATTRIBUTES,
            FD_FILESIZE, FD_PROGRESSUI, FD_UNICODE, FILEDESCRIPTORW, FILEGROUPDESCRIPTORW,
            IDataObjectAsyncCapability, IDataObjectAsyncCapability_Impl, IDragSourceHelper,
            SHCreateDataObject, SHCreateStdEnumFmtEtc, SHCreateStreamOnFileEx, SHDRAGIMAGE,
        },
    },
    core::{HRESULT, HSTRING, PCWSTR, implement},
};

#[cfg(target_os = "windows")]
#[implement(IDataObject, IDataObjectAsyncCapability)]
struct Virtual {
    items: Vec<(String, bool)>,
    promise: Arc<Promise>,
    owner: std::thread::ThreadId,
    inner: IDataObject,
    listed: Mutex<Option<Vec<Entry>>>,
    descriptor: u16,
    contents: u16,
    working: std::sync::atomic::AtomicBool,
}

#[cfg(target_os = "windows")]
impl Virtual {
    fn listed(&self) -> windows::core::Result<std::sync::MutexGuard<'_, Option<Vec<Entry>>>> {
        let mut listed = self.listed.lock().unwrap_or_else(|e| e.into_inner());
        if listed.is_none() {
            let tops = if std::thread::current().id() == self.owner {
                self.promise.ready()
            } else {
                self.promise.wait()
            };
            let Some(tops) = tops else {
                return Err(windows::core::Error::new(E_FAIL, HSTRING::new()));
            };
            *listed = Some(entries(&tops));
        }
        Ok(listed)
    }

    fn preview(&self) -> Vec<Entry> {
        self.items
            .iter()
            .map(|(name, dir)| Entry {
                name: name.clone(),
                path: PathBuf::new(),
                dir: *dir,
                size: 0,
            })
            .collect()
    }

    fn ours(&self, format: &FORMATETC) -> bool {
        format.dwAspect == DVASPECT_CONTENT.0
            && ((format.cfFormat == self.descriptor && format.tymed & TYMED_HGLOBAL.0 as u32 != 0)
                || (format.cfFormat == self.contents && format.tymed & TYMED_ISTREAM.0 as u32 != 0))
    }

    fn group(&self) -> windows::core::Result<HGLOBAL> {
        let preview;
        let listed = self.listed();
        let entries = match &listed {
            Ok(listed) => listed.as_deref().unwrap_or_default(),
            Err(_) if std::thread::current().id() == self.owner => {
                preview = self.preview();
                &preview
            }
            Err(e) => return Err(e.clone()),
        };
        let size = std::mem::size_of::<FILEGROUPDESCRIPTORW>()
            + entries.len().saturating_sub(1) * std::mem::size_of::<FILEDESCRIPTORW>();
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, size)?;
            let group = GlobalLock(handle) as *mut FILEGROUPDESCRIPTORW;
            if group.is_null() {
                return Err(windows::core::Error::new(E_FAIL, HSTRING::new()));
            }
            std::ptr::write_bytes(group as *mut u8, 0, size);
            (*group).cItems = entries.len() as u32;
            let first = std::ptr::addr_of_mut!((*group).fgd) as *mut FILEDESCRIPTORW;
            for (index, entry) in entries.iter().enumerate() {
                let mut name = [0u16; 260];
                for (slot, unit) in name.iter_mut().zip(entry.name.encode_utf16().take(259)) {
                    *slot = unit;
                }
                let item = FILEDESCRIPTORW {
                    dwFlags: (FD_ATTRIBUTES.0 | FD_FILESIZE.0 | FD_PROGRESSUI.0 | FD_UNICODE.0)
                        as u32,
                    dwFileAttributes: if entry.dir {
                        FILE_ATTRIBUTE_DIRECTORY.0
                    } else {
                        FILE_ATTRIBUTE_NORMAL.0
                    },
                    nFileSizeHigh: (entry.size >> 32) as u32,
                    nFileSizeLow: entry.size as u32,
                    cFileName: name,
                    ..Default::default()
                };
                std::ptr::write_unaligned(first.add(index), item);
            }
            let _ = GlobalUnlock(handle);
            Ok(handle)
        }
    }

    fn stream(&self, index: i32) -> windows::core::Result<windows::Win32::System::Com::IStream> {
        let listed = self.listed()?;
        let entry = usize::try_from(index)
            .ok()
            .and_then(|i| listed.as_deref().unwrap_or_default().get(i))
            .filter(|e| !e.dir)
            .ok_or_else(|| windows::core::Error::new(DV_E_FORMATETC, HSTRING::new()))?;
        let wide = HSTRING::from(entry.path.as_os_str());
        unsafe {
            SHCreateStreamOnFileEx(
                PCWSTR(wide.as_ptr()),
                (STGM_READ | STGM_SHARE_DENY_WRITE).0,
                FILE_ATTRIBUTE_NORMAL.0,
                false,
                None,
            )
        }
    }
}

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
impl IDataObject_Impl for Virtual {
    fn GetData(&self, pformatetc: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let Some(format) = (unsafe { pformatetc.as_ref() }) else {
            return Err(windows::core::Error::new(DV_E_FORMATETC, HSTRING::new()));
        };
        if !self.ours(format) {
            return unsafe { self.inner.GetData(pformatetc) };
        }
        if format.cfFormat == self.descriptor {
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 {
                    hGlobal: self.group()?,
                },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            })
        } else {
            Ok(STGMEDIUM {
                tymed: TYMED_ISTREAM.0 as u32,
                u: STGMEDIUM_0 {
                    pstm: std::mem::ManuallyDrop::new(Some(self.stream(format.lindex)?)),
                },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            })
        }
    }

    fn GetDataHere(
        &self,
        _pformatetc: *const FORMATETC,
        _pmedium: *mut STGMEDIUM,
    ) -> windows::core::Result<()> {
        Err(windows::core::Error::new(DV_E_FORMATETC, HSTRING::new()))
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        match unsafe { pformatetc.as_ref() } {
            Some(format) if self.ours(format) => S_OK,
            _ => unsafe { self.inner.QueryGetData(pformatetc) },
        }
    }

    fn GetCanonicalFormatEtc(
        &self,
        _pformatectin: *const FORMATETC,
        pformatetcout: *mut FORMATETC,
    ) -> HRESULT {
        if let Some(out) = unsafe { pformatetcout.as_mut() } {
            out.ptd = std::ptr::null_mut();
        }
        E_NOTIMPL
    }

    fn SetData(
        &self,
        pformatetc: *const FORMATETC,
        pmedium: *const STGMEDIUM,
        frelease: BOOL,
    ) -> windows::core::Result<()> {
        unsafe { self.inner.SetData(pformatetc, pmedium, frelease) }
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> windows::core::Result<IEnumFORMATETC> {
        if dwdirection != DATADIR_GET.0 as u32 {
            return Err(windows::core::Error::new(E_NOTIMPL, HSTRING::new()));
        }
        let format = |cf: u16, tymed: u32| FORMATETC {
            cfFormat: cf,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed,
        };
        let formats = [
            format(self.descriptor, TYMED_HGLOBAL.0 as u32),
            format(self.contents, TYMED_ISTREAM.0 as u32),
        ];
        unsafe { SHCreateStdEnumFmtEtc(&formats) }
    }

    fn DAdvise(
        &self,
        _pformatetc: *const FORMATETC,
        _advf: u32,
        _padvsink: Option<&IAdviseSink>,
    ) -> windows::core::Result<u32> {
        Err(windows::core::Error::new(
            OLE_E_ADVISENOTSUPPORTED,
            HSTRING::new(),
        ))
    }

    fn DUnadvise(&self, _dwconnection: u32) -> windows::core::Result<()> {
        Err(windows::core::Error::new(
            OLE_E_ADVISENOTSUPPORTED,
            HSTRING::new(),
        ))
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(windows::core::Error::new(
            OLE_E_ADVISENOTSUPPORTED,
            HSTRING::new(),
        ))
    }
}

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
impl IDataObjectAsyncCapability_Impl for Virtual {
    fn SetAsyncMode(&self, _fdoopasync: BOOL) -> windows::core::Result<()> {
        Ok(())
    }

    fn GetAsyncMode(&self) -> windows::core::Result<BOOL> {
        Ok(true.into())
    }

    fn StartOperation(&self, _pbcreserved: Option<&IBindCtx>) -> windows::core::Result<()> {
        self.working
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn InOperation(&self) -> windows::core::Result<BOOL> {
        Ok(self
            .working
            .load(std::sync::atomic::Ordering::SeqCst)
            .into())
    }

    fn EndOperation(
        &self,
        _hresult: HRESULT,
        _pbcreserved: Option<&IBindCtx>,
        _dweffects: u32,
    ) -> windows::core::Result<()> {
        self.working
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(target_os = "windows")]
#[implement(IDropSource)]
struct Source;

#[cfg(target_os = "windows")]
#[allow(non_snake_case)]
impl IDropSource_Impl for Source {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if fescapepressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if grfkeystate & MK_LBUTTON == MODIFIERKEYS_FLAGS(0) {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(target_os = "windows")]
fn bitmap(png: &[u8]) -> Option<SHDRAGIMAGE> {
    let decoder = ::png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut rgba).ok()?;
    let (width, height) = (info.width as i32, info.height as i32);
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let handle =
        unsafe { CreateDIBSection(None, &header, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        return None;
    }
    let out =
        unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (width * height * 4) as usize) };
    for (target, source) in out.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
        let alpha = source[3] as u32;
        target[0] = (source[2] as u32 * alpha / 255) as u8;
        target[1] = (source[1] as u32 * alpha / 255) as u8;
        target[2] = (source[0] as u32 * alpha / 255) as u8;
        target[3] = source[3];
    }
    Some(SHDRAGIMAGE {
        sizeDragImage: SIZE {
            cx: width,
            cy: height,
        },
        ptOffset: POINT {
            x: width / 2,
            y: height / 2,
        },
        hbmpDragImage: handle,
        crColorKey: COLORREF(0xFFFF_FFFF),
    })
}

#[cfg(target_os = "windows")]
pub fn start(
    _frame: &eframe::Frame,
    items: Vec<(String, bool)>,
    promise: Arc<Promise>,
    icon: &[u8],
    done: impl Fn(bool) + Send + 'static,
) -> Result<(), String> {
    unsafe {
        let _ = OleInitialize(None);
        let inner: IDataObject = SHCreateDataObject(None, None, None).map_err(|e| e.to_string())?;
        let virtual_files = Virtual {
            items,
            promise,
            owner: std::thread::current().id(),
            inner,
            listed: Mutex::new(None),
            descriptor: RegisterClipboardFormatW(CFSTR_FILEDESCRIPTORW) as u16,
            contents: RegisterClipboardFormatW(CFSTR_FILECONTENTS) as u16,
            working: std::sync::atomic::AtomicBool::new(false),
        };
        let data: IDataObject = virtual_files.into();
        if let Some(image) = bitmap(icon)
            && let Ok(helper) =
                CoCreateInstance::<_, IDragSourceHelper>(&CLSID_DragDropHelper, None, CLSCTX_ALL)
        {
            let _ = helper.InitializeFromBitmap(&image, &data);
        }
        let source: IDropSource = Source.into();
        let mut effect = DROPEFFECT::default();
        let result = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
        done(result == DRAGDROP_S_DROP);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApp, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSEventModifierFlags, NSEventType, NSFilePromiseProvider,
    NSFilePromiseProviderDelegate, NSImage, NSView,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{
    NSArray, NSData, NSError, NSOperationQueue, NSPoint, NSRect, NSString, NSURL,
};

#[cfg(target_os = "macos")]
struct Promised {
    name: String,
    promise: Arc<Promise>,
    queue: Retained<NSOperationQueue>,
}

#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "TrorPadFilePromise"]
    #[ivars = Promised]
    struct FilePromise;

    unsafe impl NSObjectProtocol for FilePromise {}

    unsafe impl NSFilePromiseProviderDelegate for FilePromise {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn file_name(
            &self,
            _provider: &NSFilePromiseProvider,
            _kind: &NSString,
        ) -> Retained<NSString> {
            NSString::from_str(&self.ivars().name)
        }

        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _provider: &NSFilePromiseProvider,
            url: &NSURL,
            completion: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let ivars = self.ivars();
            let target = url.path().map(|p| PathBuf::from(p.to_string()));
            let placed = ivars.promise.wait().zip(target).and_then(|(tops, target)| {
                let source = tops.into_iter().find(|t| {
                    t.file_name()
                        .is_some_and(|n| n.to_string_lossy() == ivars.name)
                })?;
                place(&source, &target).ok()
            });
            if placed.is_some() {
                completion.call((std::ptr::null_mut(),));
            } else {
                let error = NSError::new(-1, &NSString::from_str("TrorPad"));
                completion.call((Retained::as_ptr(&error) as *mut NSError,));
            }
        }

        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn queue(&self, _provider: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }
    }
);

#[cfg(target_os = "macos")]
struct Ending {
    done: Box<dyn Fn(bool) + Send>,
}

#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TrorPadPromiseSource"]
    #[ivars = Ending]
    struct PromiseSource;

    unsafe impl NSObjectProtocol for PromiseSource {}

    unsafe impl NSDraggingSource for PromiseSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn mask(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _point: NSPoint, operation: NSDragOperation) {
            (self.ivars().done)(operation != NSDragOperation::None);
        }
    }
);

#[cfg(target_os = "macos")]
pub fn start(
    frame: &eframe::Frame,
    items: Vec<(String, bool)>,
    promise: Arc<Promise>,
    icon: &[u8],
    done: impl Fn(bool) + Send + 'static,
) -> Result<(), String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(RawWindowHandle::AppKit(handle)) = frame.window_handle().map(|h| h.as_raw()) else {
        return Err("no window".to_owned());
    };
    let mtm = MainThreadMarker::new().ok_or("not the main thread")?;
    let view = unsafe { &*(handle.ns_view.as_ptr() as *const NSView) };
    let window = view.window().ok_or("no window")?;
    let content = window.contentView().ok_or("no view")?;
    let at = window.mouseLocationOutsideOfEventStream();
    let image =
        NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(icon)).ok_or("no image")?;
    let size = image.size();
    let frame_rect = NSRect::new(
        NSPoint::new(at.x - size.width / 2.0, at.y - size.height / 2.0),
        size,
    );
    let queue = NSOperationQueue::new();
    let mut dragged = Vec::new();
    for (name, dir) in items {
        let kind = NSString::from_str(if dir { "public.folder" } else { "public.data" });
        let delegate = FilePromise::alloc().set_ivars(Promised {
            name,
            promise: promise.clone(),
            queue: queue.clone(),
        });
        let delegate: Retained<FilePromise> = unsafe { msg_send![super(delegate), init] };
        let provider = NSFilePromiseProvider::initWithFileType_delegate(
            NSFilePromiseProvider::alloc(),
            &kind,
            ProtocolObject::from_ref(&*delegate),
        );
        unsafe { provider.setUserInfo(Some(&*(Retained::as_ptr(&delegate) as *const AnyObject))) };
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*provider),
        );
        unsafe { item.setDraggingFrame_contents(frame_rect, Some(&image)) };
        dragged.push(item);
    }
    let dragged = NSArray::from_retained_slice(&dragged);
    let timestamp = NSApp(mtm)
        .currentEvent()
        .map(|e| e.timestamp())
        .unwrap_or(0.0);
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDragged,
        at,
        NSEventModifierFlags::empty(),
        timestamp,
        window.windowNumber(),
        None,
        0,
        1,
        1.0,
    )
    .ok_or("no event")?;
    let source = PromiseSource::alloc(mtm).set_ivars(Ending {
        done: Box::new(done),
    });
    let source: Retained<PromiseSource> = unsafe { msg_send![super(source), init] };
    content.beginDraggingSessionWithItems_event_source(
        &dragged,
        &event,
        ProtocolObject::from_ref(&*source),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_side_gets_the_files_once_they_are_ready() {
        let promise = Promise::new();
        let waiter = {
            let promise = promise.clone();
            std::thread::spawn(move || promise.wait())
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(promise.ready().is_none());
        promise.fulfil(vec![PathBuf::from("a")]);
        assert_eq!(waiter.join().unwrap(), Some(vec![PathBuf::from("a")]));
        promise.fail();
        assert_eq!(promise.ready(), Some(vec![PathBuf::from("a")]));
    }

    #[test]
    fn failed_promise_releases_the_waiter() {
        let promise = Promise::new();
        let waiter = {
            let promise = promise.clone();
            std::thread::spawn(move || promise.wait())
        };
        promise.fail();
        assert_eq!(waiter.join().unwrap(), None);
    }

    #[test]
    fn folders_are_listed_before_their_files_and_placed_whole() {
        let root = std::env::temp_dir().join(format!("trorpad-promise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let site = root.join("from").join("site");
        std::fs::create_dir_all(site.join("css")).unwrap();
        std::fs::write(site.join("index.php"), "<?php").unwrap();
        std::fs::write(site.join("css").join("a.css"), "a{}").unwrap();
        let listed: Vec<(String, bool, u64)> = entries(std::slice::from_ref(&site))
            .into_iter()
            .map(|e| (e.name, e.dir, e.size))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("site".to_owned(), true, 0),
                ("site\\css".to_owned(), true, 0),
                ("site\\css\\a.css".to_owned(), false, 3),
                ("site\\index.php".to_owned(), false, 5),
            ]
        );
        let target = root.join("to").join("site");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        place(&site, &target).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("css").join("a.css")).unwrap(),
            "a{}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
