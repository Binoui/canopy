use std::{
    future::Future,
    marker::PhantomData,
    path::Path,
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use glib::{thread_guard::ThreadGuard, translate::*};
use webkit2gtk::{ffi, UserContentManager};

glib::wrapper! {
    struct FilterObject(Shared<ffi::WebKitUserContentFilter>);

    match fn {
        ref => |ptr| ffi::webkit_user_content_filter_ref(ptr),
        unref => |ptr| ffi::webkit_user_content_filter_unref(ptr),
        type_ => || ffi::webkit_user_content_filter_get_type(),
    }
}

glib::wrapper! {
    struct StoreObject(Object<ffi::WebKitUserContentFilterStore, ffi::WebKitUserContentFilterStoreClass>);

    match fn {
        type_ => || ffi::webkit_user_content_filter_store_get_type(),
    }
}

pub(super) struct Filter(FilterObject, PhantomData<Rc<()>>);

impl Filter {
    pub(super) fn attach(&self, manager: &UserContentManager) {
        unsafe {
            ffi::webkit_user_content_manager_add_filter(
                manager.to_glib_none().0,
                self.0.to_glib_none().0,
            );
        }
    }
}

pub(super) fn detach(manager: &UserContentManager, identifier: &str) {
    unsafe {
        ffi::webkit_user_content_manager_remove_filter_by_id(
            manager.to_glib_none().0,
            identifier.to_glib_none().0,
        );
    }
}

#[derive(Clone)]
pub(super) struct FilterStore(StoreObject, PhantomData<Rc<()>>);

type StorePtr = *mut ffi::WebKitUserContentFilterStore;
type AsyncResult = *mut gio::ffi::GAsyncResult;
type ErrorOut = *mut *mut glib::ffi::GError;
type FinishFilter =
    unsafe extern "C" fn(StorePtr, AsyncResult, ErrorOut) -> *mut ffi::WebKitUserContentFilter;

impl FilterStore {
    pub(super) fn new(directory: &Path) -> Self {
        assert!(gtk::is_initialized_main_thread());
        let store =
            unsafe { ffi::webkit_user_content_filter_store_new(directory.to_glib_none().0) };
        assert!(
            !store.is_null(),
            "WebKit did not return a content filter store"
        );
        Self(unsafe { from_glib_full(store) }, PhantomData)
    }

    pub(super) fn load(&self, identifier: String) -> impl Future<Output = Result<Filter>> {
        self.operation(
            move |store, cancel, callback, data| unsafe {
                ffi::webkit_user_content_filter_store_load(
                    store,
                    identifier.to_glib_none().0,
                    cancel,
                    callback,
                    data,
                )
            },
            finish_filter(ffi::webkit_user_content_filter_store_load_finish),
        )
    }

    pub(super) fn save(
        &self,
        identifier: String,
        rules: glib::Bytes,
        call_time: impl FnOnce(Duration) + 'static,
    ) -> impl Future<Output = Result<Filter>> {
        self.operation(
            move |store, cancel, callback, data| {
                let identifier = identifier.to_glib_none();
                let started = Instant::now();
                unsafe {
                    ffi::webkit_user_content_filter_store_save(
                        store,
                        identifier.0,
                        rules.to_glib_none().0,
                        cancel,
                        callback,
                        data,
                    );
                }
                call_time(started.elapsed());
            },
            finish_filter(ffi::webkit_user_content_filter_store_save_finish),
        )
    }

    pub(super) fn identifiers(&self) -> impl Future<Output = Option<Vec<String>>> {
        self.operation(
            |store, cancel, callback, data| unsafe {
                ffi::webkit_user_content_filter_store_fetch_identifiers(
                    store, cancel, callback, data,
                )
            },
            |store, result| unsafe {
                let names =
                    ffi::webkit_user_content_filter_store_fetch_identifiers_finish(store, result);
                (!names.is_null()).then(|| FromGlibPtrContainer::from_glib_full(names))
            },
        )
    }

    pub(super) fn remove(&self, identifier: String) -> impl Future<Output = ()> {
        self.operation(
            move |store, cancel, callback, data| unsafe {
                ffi::webkit_user_content_filter_store_remove(
                    store,
                    identifier.to_glib_none().0,
                    cancel,
                    callback,
                    data,
                )
            },
            |store, result| unsafe {
                ffi::webkit_user_content_filter_store_remove_finish(store, result, ptr::null_mut());
            },
        )
    }

    fn operation<T: 'static>(
        &self,
        start: impl FnOnce(
                StorePtr,
                *mut gio::ffi::GCancellable,
                gio::ffi::GAsyncReadyCallback,
                glib::ffi::gpointer,
            ) + 'static,
        finish: impl FnOnce(StorePtr, AsyncResult) -> T + 'static,
    ) -> impl Future<Output = T> {
        type Done = Box<dyn FnOnce(StorePtr, AsyncResult)>;

        unsafe extern "C" fn complete(
            source: *mut glib::gobject_ffi::GObject,
            result: AsyncResult,
            data: glib::ffi::gpointer,
        ) {
            let done = unsafe { Box::from_raw(data.cast::<ThreadGuard<Done>>()) }.into_inner();
            done(source.cast(), result);
        }

        gio::GioFuture::new(self, move |store, cancel, send| {
            let done: Done = Box::new(move |store, result| send.resolve(finish(store, result)));
            // GTask retains the source and calls back once on this thread, including on cancel.
            let data = Box::into_raw(Box::new(ThreadGuard::new(done))).cast();
            start(
                store.0.to_glib_none().0,
                cancel.to_glib_none().0,
                Some(complete),
                data,
            );
        })
    }
}

fn finish_filter(finish: FinishFilter) -> impl FnOnce(StorePtr, AsyncResult) -> Result<Filter> {
    move |store, result| unsafe {
        let mut error = ptr::null_mut();
        let filter: Option<FilterObject> = from_glib_full(finish(store, result, &mut error));
        if !error.is_null() {
            return Err(from_glib_full::<_, glib::Error>(error).into());
        }
        filter
            .map(|filter| Filter(filter, PhantomData))
            .context("WebKit did not return a content filter.")
    }
}
