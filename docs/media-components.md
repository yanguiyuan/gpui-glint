# Media components

These components use GPUI image sources and the host's registered HTTP client. They do not know about collections, chapters, service credentials or image-restoration protocols.

## ImageItem, ImageStream and ImageViewer

`ImageItem` carries a stable id, an `ImageSource` and optional original dimensions. Sources can be URLs, filesystem resources, decoded images, encoded images or custom loaders. Supply dimensions when known to avoid placeholder geometry before decode.

```rust,ignore
let pages = vec![ImageItem::new("page-1", "https://example.test/1.jpg").dimensions(1080, 1920)];
let stream = cx.new(ImageStream::new);
stream.update(cx, |stream, cx| {
    stream.set_theme(theme, cx);
    stream.set_window(StreamWindow { before: 2, after: 3 }, cx);
    stream.set_items(pages.clone(), 0, cx);
    stream.scroll_to_id("page-1", cx);
});
let single = ImageViewer::from_item(pages[0].clone()).theme(theme).padding(16.).render();
```

`ImageStream` renders only pages intersecting the viewport. `StreamWindow` independently controls decoded-image retention and preloading around it (default: five pages before and five after; use zero for strict viewport-only loading). Image dimensions are measured after decode, with a fractional page anchor retained during resize/reflow. Resource textures outside the retention window are released. The component emits `ImageStreamEvent::PageChanged(index)`; page numbering, keyboard bindings, persistence and transformations stay with the host.

`ImageViewer` provides a bounded single-image surface with contain fitting and themed loading/error states. Its optional `toolbar` accepts host controls, and `ImageViewer::stage` can host custom loading, empty and retry states without allowing image intrinsic dimensions to stretch a flex layout.

## ThumbnailList

```rust,ignore
let list = cx.new(ThumbnailList::new);
list.update(cx, |list, cx| {
    list.set_items(vec![ThumbnailItem::new("document-1", "Document")
        .subtitle("Updated today").source(std::path::PathBuf::from("/path/to/thumbnail.png"))
        .placeholder("DOC").trailing("3")], cx);
    list.set_selected(Some("document-1".into()), cx);
    list.set_theme(theme, cx);
});
```

Rows have a fixed height and only visible thumbnails are instantiated. Selection is controlled by the host through `Selected(id)`. Enable `set_has_more(true)` to receive one `LoadMore` event near the end. Appending items re-arms pagination; disable it on completion or failure. The host owns fetching, retry UI and its scroll handle.

## NavigationRail

```rust,ignore
NavigationRail::new([])
    .groups([
        NavigationGroup::new("agent", [NavigationItem::new("chat", "Chat", Icon::MessageCircle)]),
        NavigationGroup::new("api", [NavigationItem::new("requests", "Requests", Icon::Folders)]),
    ])
    .footer_group(NavigationGroup::new("common", [NavigationItem::new("settings", "Settings", Icon::Settings)]))
    .selected("chat").theme(theme)
    .on_select(|id, window, cx| { /* map the id to a host route or action */ })
    .render(cx)
```

Non-empty groups are separated by a divider; footer actions remain pinned to the bottom. Empty groups add no divider. `new(items)` and `footer(items)` are convenience forms for a single primary group and footer. Stable item ids also identify buttons for automation. Routes, action semantics and selected state belong to the host. Workbench uses Agent, API and common groups, including its new-session action.

## ImageByteCache (infrastructure)

`gpui_glint::image_cache::ImageByteCache<K, V>` is independent of networking and image decoding. Implement `ImageCacheValue` on immutable values to report byte size and whether a response is cacheable. Construct separate caches for original downloads and transformed results if their budgets should be independent.

- Both byte and entry budgets bound completed values; LRU eviction applies.
- `get_or_load` shares an in-flight future between callers with the same key.
- Keys should include an identity namespace and transform fingerprint when relevant.
- Failed responses and oversized values can be returned without retention. Errors are not cached.
- Authentication, request headers, backend choice, retry policy and transforms remain host responsibilities.

The playground's **图片阅读与缩略图** page demonstrates all four UI components with local in-memory images and no server.
