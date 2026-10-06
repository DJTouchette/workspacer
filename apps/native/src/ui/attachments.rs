//! Draft attachments: picking and pasting files, their upload to the hub,
//! and the chips and thumbnails above the composer.
use super::*;
use wks_native::features::{AttachmentSource, Request};

/// A draft attachment: name, hub path, and its thumbnail (`None` loading,
/// `Some(None)` unavailable or not an image).
pub(super) type DraftFile = (String, String, Option<Option<Arc<gpui::Image>>>);

impl Workspace {
    /// Finished uploads join their session's draft; an acknowledged send
    /// retires the draft and attachments it carried, and a refused one
    /// leaves them for another try.
    pub(super) fn sync_attachments(
        &mut self,
        next: &View,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = next.requests.get("upload")
            && !state.loading
            && state.number > self.extras.upload_receipt
        {
            self.extras.upload_receipt = state.number;
            if let Some(error) = &state.error {
                self.extras.notice = format!("Attachment failed: {error}");
            } else if let Request::Upload { session, .. } = &state.request
                && let (Some(name), Some(path)) =
                    (state.value["name"].as_str(), state.value["path"].as_str())
            {
                self.extras
                    .attachments
                    .entry(session.clone())
                    .or_default()
                    .push((name.into(), path.into()));
            }
        }
        if let Some(receipt) = &next.receipt
            && receipt.number > self.last_receipt
        {
            if receipt.error.is_none() && matches!(receipt.action, Action::Send(_)) {
                if let Some(sent) = self.extras.sent_drafts.remove(&receipt.session) {
                    if self.view.selected.as_ref() == Some(&receipt.session) {
                        if self.composer.read(cx).value().as_ref() == sent {
                            self.composer
                                .update(cx, |i, cx| i.set_value("", window, cx));
                        }
                    } else if self.drafts.get(&receipt.session) == Some(&sent) {
                        self.drafts.remove(&receipt.session);
                    }
                }
                if let Some(sent) = self.extras.sent_attachments.remove(&receipt.session)
                    && let Some(draft) = self.extras.attachments.get_mut(&receipt.session)
                {
                    draft.retain(|item| !sent.contains(item));
                }
            } else {
                self.extras.sent_attachments.remove(&receipt.session);
                self.extras.sent_drafts.remove(&receipt.session);
            }
        }
    }

    pub(super) fn pick_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.view.selected.clone() else {
            return;
        };
        if self.uploading() {
            return;
        }
        let pick = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Attach image or PDF (up to 8 MiB)".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = pick.await;
            let _ = this.update_in(cx, |this, _, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.first() {
                        this.request(
                            Request::Upload {
                                session,
                                source: AttachmentSource::File(path.clone()),
                            },
                            cx,
                        );
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    this.extras.notice = "Could not open the attachment picker.".into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn paste_image(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.view.selected.clone() else {
            return false;
        };
        if self.uploading() {
            return false;
        }
        if let Some(clipboard) = cx.read_from_clipboard() {
            if clipboard.text().is_some() {
                return false;
            }
            for entry in clipboard.entries() {
                if let gpui::ClipboardEntry::Image(image) = entry {
                    let name = format!(
                        "Screenshot.{}",
                        match image.format() {
                            gpui::ImageFormat::Png => "png",
                            gpui::ImageFormat::Jpeg => "jpg",
                            gpui::ImageFormat::Webp => "webp",
                            gpui::ImageFormat::Gif => "gif",
                            gpui::ImageFormat::Tiff => "tiff",
                            gpui::ImageFormat::Bmp => "bmp",
                            _ => {
                                self.extras.notice =
                                    "Paste a PNG, JPEG, GIF, or WebP screenshot.".into();
                                cx.notify();
                                return true;
                            }
                        }
                    );
                    if image.bytes().len() > wks_native::features::MAX_ATTACHMENT_BYTES {
                        self.extras.notice =
                            "Screenshot exceeds 8 MiB. Save a smaller image and attach it.".into();
                        cx.notify();
                        return true;
                    }
                    self.request(
                        Request::Upload {
                            session,
                            source: AttachmentSource::Image {
                                name,
                                bytes: Arc::new(image.bytes().to_vec()),
                            },
                        },
                        cx,
                    );
                    return true;
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            self.request(
                Request::Upload {
                    session,
                    source: AttachmentSource::WindowsClipboard,
                },
                cx,
            );
            true
        }
        #[cfg(not(target_os = "windows"))]
        false
    }

    /// Draft attachments above the composer: images as thumbnails (read back
    /// from the hub, so they show the uploaded file), PDFs and images whose
    /// preview is loading or unavailable as named chips. Each keeps Remove;
    /// a thumbnail opens the image in the viewer.
    pub(super) fn render_draft_attachments(
        &self,
        files: Vec<DraftFile>,
        cwd: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.appearance.palette();
        let busy = self.view.busy;
        let remove = |this: &Self, ix: usize, cx: &mut Context<Self>| {
            this.icon_button(
                "remove-attachment",
                "Remove attachment",
                IconName::Close,
                !busy,
            )
            .size(px(24.))
            .when(!busy, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(id) = &this.view.selected
                        && let Some(files) = this.extras.attachments.get_mut(id)
                        && ix < files.len()
                    {
                        files.remove(ix);
                    }
                    cx.notify();
                }))
            })
        };
        let uploading = self.uploading()
            && self.view.requests.get("upload").is_some_and(|state| {
                matches!(&state.request, Request::Upload { session, .. }
                    if Some(session) == self.view.selected.as_ref())
            });
        div()
            .flex()
            .flex_wrap()
            .items_end()
            .gap_2()
            .children(
                files
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (name, path, preview))| {
                        match preview {
                            Some(Some(image)) => {
                                let link = wks_native::links::tool_file(cwd, &path, None);
                                let owner = self.view.selected.clone().unwrap_or_default();
                                div()
                                    .id(("attachment", ix))
                                    .debug_selector(move || format!("draft-thumbnail-{ix}"))
                                    .relative()
                                    .rounded_md()
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(rgb(p.border))
                                    .bg(rgb(p.base))
                                    .child(
                                        div()
                                            .id(("attachment-preview", ix))
                                            .cursor_pointer()
                                            .tooltip({
                                                let name = SharedString::from(name.clone());
                                                move |window, cx| {
                                                    gpui_component::tooltip::Tooltip::new(
                                                        name.clone(),
                                                    )
                                                    .build(window, cx)
                                                }
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_link(&owner, link.clone(), cx)
                                            }))
                                            .child(
                                                // A fixed tile: the decode is async, and
                                                // the row must not jump when it lands.
                                                gpui::img(image)
                                                    .w(px(96.))
                                                    .h(px(72.))
                                                    .object_fit(gpui::ObjectFit::Cover),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .top(px(2.))
                                            .right(px(2.))
                                            .rounded_full()
                                            .bg(rgb(p.surface))
                                            .child(remove(self, ix, cx)),
                                    )
                            }
                            preview => div()
                                .id(("attachment", ix))
                                .debug_selector(move || format!("draft-attachment-{ix}"))
                                .pl_2()
                                .pr_1()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(p.selected))
                                .max_w_full()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(match preview {
                                    None => brand_spinner(
                                        12.,
                                        p,
                                        SharedString::from(format!("draft-preview-{ix}")),
                                    )
                                    .into_any_element(),
                                    Some(_) => Icon::new(IconName::File)
                                        .size(px(14.))
                                        .text_color(rgb(p.accent))
                                        .into_any_element(),
                                })
                                .child(div().min_w_0().truncate().text_size(px(12.)).child(name))
                                .child(remove(self, ix, cx)),
                        }
                    }),
            )
            .when(uploading, |d| {
                d.child(
                    div()
                        .debug_selector(|| "draft-uploading".into())
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(rgb(p.selected))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(p.muted))
                        .child(brand_spinner(12., p, "draft-uploading-spinner"))
                        .child("Attaching…"),
                )
            })
    }

    pub(super) fn uploading(&self) -> bool {
        self.view.requests.get("upload").is_some_and(|s| s.loading)
    }

    pub(super) fn attachment_text(&self, id: &str, text: &str) -> String {
        let mut result = String::new();
        for (name, path) in self.extras.attachments.get(id).into_iter().flatten() {
            let kind = if name.to_lowercase().ends_with(".pdf") {
                "PDF"
            } else {
                "Image"
            };
            result.push_str(&format!("[{kind}: {path}]\n"));
        }
        result.push_str(text);
        result
    }
}
