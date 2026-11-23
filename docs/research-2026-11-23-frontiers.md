# shadcn-rs — Nghiên cứu các bài toán chưa ai giải

2026-11-23. Tài liệu ghi nghiên cứu sâu các bài toán còn mở của ngành mà `shadcn-rs` có thể là người đầu tiên giải, theo
nguyên tắc "Be the first to solve it" và "Read the sources for their problems" trong `AGENTS.md`. Nó nối tiếp mục 18 và
danh sách chín bài toán trong `docs/review-2026-11-23.md`. Đây là nghiên cứu, không phải ADR: không quyết định nào ở đây
đã được chấp nhận.

## Mục lục

1. [Cách đọc](#1-cách-đọc)
2. [Phát hiện chung: frame rẽ nhánh là nền móng](#2-phát-hiện-chung-frame-rẽ-nhánh-là-nền-móng)
3. [Độ trễ từ input tới pixel: vẽ trước theo công thức](#3-độ-trễ-từ-input-tới-pixel-vẽ-trước-theo-công-thức)
4. [Chạy lại tất định toàn bộ app](#4-chạy-lại-tất-định-toàn-bộ-app)
5. [App mà AI agent vận hành được](#5-app-mà-ai-agent-vận-hành-được)
6. [Tính tăng dần chứng minh được](#6-tính-tăng-dần-chứng-minh-được)
7. [Dữ liệu cực lớn: list, bảng, chart](#7-dữ-liệu-cực-lớn-list-bảng-chart)
8. [Chữ đẹp hơn trình duyệt](#8-chữ-đẹp-hơn-trình-duyệt)
9. [Màu đúng từ đầu tới cuối, gam màu rộng và HDR](#9-màu-đúng-từ-đầu-tới-cuối-gam-màu-rộng-và-hdr)
10. [Điện năng là chỉ số hạng nhất](#10-điện-năng-là-chỉ-số-hạng-nhất)
11. [Nhiều màn hình, mỗi màn hình một nhịp](#11-nhiều-màn-hình-mỗi-màn-hình-một-nhịp)
12. [Trạng thái](#12-trạng-thái)
13. [Quyết định](#13-quyết-định)

---

## 1. Cách đọc

- Mỗi bài do một agent nghiên cứu: đọc mã nguồn trong `references/` và trong repo, đọc ticket, bài báo, bài viết kỹ
  thuật. Không build, không chạy, không đăng gì.
- **[V]**: agent đã đọc nguồn, trang hoặc ticket. **[I]**: suy luận, trí nhớ hoặc nguồn thứ cấp.
- **Kiểm chứng chéo:** mỗi bài được một agent Codex (khác họ model) kiểm lại độc lập, chỉ đọc. Chỗ Codex bác hoặc sửa
  được ghi "[V, Codex]" kèm điều đúng; bản sai cũ được nêu rõ thay vì xoá lặng lẽ.
- "Đầu tiên trong ngành" nghĩa là agent không tìm thấy tiền lệ; tìm kiếm không vét cạn.
- Mỗi bài theo cùng khuôn: vấn đề và bằng chứng; vì sao những bên khác chưa giải (giới hạn cũ); ta có gì mà họ không
  có; thiết kế đề xuất; thí nghiệm đầu tiên; rủi ro.

## 2. Phát hiện chung: frame rẽ nhánh là nền móng

Bốn bài nghiên cứu độc lập cùng dẫn về một cơ chế đã có trong code: `crates/gui/src/engine.rs` rẽ nhánh được cả arena
reactive lẫn cây instance qua một undo log, giữ hàng đợi timer và mailbox riêng, rồi khôi phục khi bỏ nhánh. Hiện nó chỉ
phục vụ cold-check ở bản debug. gui cấm `thread_local!` thô (`crates/gui/clippy.toml:9-12`); TLS nằm trong
`engine_local!` (30 khối macro trong `crates/gui/src`, một khối khai báo được nhiều slot) có nhãn Fork, Fresh hoặc Shared
[V, Codex kiểm lại; bản trước ghi "39" là sai].

**Nhưng registry không liệt kê hết state** [V, Codex]. Nằm ngoài nó: state dùng chung có tính khả biến bên trong của
`ListState` (`list/virtual_list.rs:186,268`), interner style toàn process (`style.rs:984`), sender mailbox ghi thẳng
vào channel (`mailbox.rs:50`), bộ đếm id series và page toàn process (`chunk/series.rs:717-718`) mà một nhánh có thể đẩy
tăng. Vậy cô lập của nhánh và tính đầy đủ của replay đều chưa được bảo đảm theo cấu trúc; phải đưa những state này vào
registry hoặc chứng minh chúng vô hại, và cần một luật lint bắt state toàn process mới.

| Bài | Dùng frame rẽ nhánh để |
|---|---|
| Độ trễ input | Chạy chính code của component trên một input giả định, lấy kết quả làm "công thức" cho render thread vẽ trước |
| Chạy lại tất định | Lùi từng bước trong W frame gần nhất bằng undo log, không cần chụp lại cả state |
| App cho AI agent | Xem trước hậu quả của một hành động: chạy trên nhánh, trả về diff, rồi commit hoặc bỏ |
| Chứng minh tính tăng dần | Cold check lấy mẫu trên bản release: lúc rảnh chạy một frame lạnh trên nhánh, so với frame tăng dần, lệch thì xuất bản ghi replay |

Hệ quả: **đưa frame rẽ nhánh lên bản release, kèm một "outbox" giữ lại mọi tác dụng ra ngoài app**, là việc nền chung
của cả bốn. Mọi state chưa đăng ký là `engine_local!` và mọi tác dụng phụ đi vòng qua outbox đều phá cô lập; đây là rủi
ro chung cần một ADR riêng.

---

## 3. Độ trễ từ input tới pixel: vẽ trước theo công thức

**Mục tiêu.** Pixel đầu tiên phản ánh một lần bấm phím, click, kéo hay cuộn xuất hiện ở vblank kế, thấp hơn mọi trình
duyệt và toolkit native, không sai hình.

### 3.1 Độ trễ đến từ đâu

| Khâu | Số đo |
|---|---|
| Bàn phím | 15 ms (Apple Magic Keyboard) tới 60 ms (Logitech K360), camera 240 fps và logic analyzer USB ([Dan Luu](https://danluu.com/keyboard-latency/)) [V] |
| macOS trong cửa sổ | Zed đo từ phím tới pixel đầu tiên đổi (ScreenCaptureKit, M1 Pro, 120 Hz): CVDisplayLink trung vị 27,0/27,3 ms, `CAMetalDisplayLink` 44,6/45,0 ms (nhanh nhất 16,9 so với 38,0 ms); một app GPUI riêng thấy callback mỗi 8,2 ms nhắm trước 41,6 ms, không đổi khi `preferredFrameLatency` từ 2 xuống 1; tắt display sync còn khoảng 27 ms nhưng p95 cuộn 8,4 → 15,8 ms và rớt khoảng 12% frame ([zed#66954](https://github.com/zed-industries/zed/issues/66954)) [V, Codex]. Đây là regression trên máy đã thử, không chứng minh nguyên nhân chung hay một sàn cố định của WindowServer. PR dùng `CAMetalDisplayLink` của wgpu (#10643) bị đóng [V] |
| Windows | Microsoft ghi độ trễ một frame đạt được ở Independent Flip trên Windows mới, rơi về composed khi không đủ điều kiện (không có con số "thêm đúng một frame" cố định); `ALLOW_TEARING` thấp hơn trên cấu hình đủ điều kiện, kể cả cửa sổ có multiplane overlay [V, Codex] ([MS Learn](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model)) [V] |
| Linux | KWin thức dậy lúc `max(target_present − ước lượng thời gian composite, now)`; ước lượng ban đầu là dự đoán từ journal cộng margin, kẹp tối đa 2 interval; đích present tính riêng (`references/kwin/src/core/renderloop.cpp:54`) [V, Codex sửa]; tearing chỉ cho surface toàn màn hình đang hoạt động và các điều kiện khác (`compositor.cpp:809`) [V] |
| Nhịp frame | Raph Levien: ví dụ minh hoạ khoảng 8 ms phí vì hai lần chờ sai nhịp; một trace Android thật, mà tác giả ước có thể từ 37 xuống 17 ms (từ xử lý input tới bắt đầu scanout) nếu bắt đầu vẽ đúng lúc ([blog](https://raphlinus.github.io/ui/graphics/gpu/2021/10/22/swapchain-frame-pacing.html)) [V] |
| Cảm nhận | Ng và cộng sự (UIST '12): người dùng vẫn phân biệt được độ trễ dưới 10 ms; đề xuất phản hồi thô ngay rồi vẽ đầy đủ sau ([ACM](https://dl.acm.org/doi/10.1145/2380116.2380174)) [V tóm tắt] |

**Phần của ta.** Input phải đi platform → UI → layout và paint → commit trước `deadline = present − margin`
(`compositor/thread.rs`). Ở 120 Hz với UI frame tốn 4 ms, ước khoảng một nửa số lần bấm phím lỡ vblank kế và mất thêm
8,3 ms [I, tính toán]. Vẽ trước theo công thức dời ngưỡng cắt input từ `present − (UI + vẽ + margin)` sang
`present − (vẽ + margin)`.

### 3.2 Tiền lệ

- **IntelliJ "zero-latency typing"** (bật mặc định từ 2017.1): Windows 2,9 ms so với 24,7 ms; Linux 1,7 ms so với
  69,2 ms (đo framebuffer bằng Typometer). `ImmediatePainter.java` vẽ ký tự và caret trước khi document cập nhật, chỉ
  khi: một caret, không selection, caret dời một vị trí, thay một đơn vị UTF-16, ở cuối dòng hoặc cuối document, có lexer
  highlighter, không virtual space, không RTL hay ranh giới bidi, không border effect, không letter spacing, không chữ
  nhô đè dòng, cộng kiểm loại editor, style và clip. Tỉ lệ màn hình lẻ chỉ tắt *double buffering* của bản vẽ tức thì,
  vẽ tức thì vẫn chạy ([ImmediatePainter](https://github.com/JetBrains/intellij-community/blob/76505524be37de58d4a55a70191f0b041e54a8d6/platform/platform-impl/src/com/intellij/openapi/editor/impl/ImmediatePainter.java#L86))
  [V, Codex sửa]. Đây là bằng chứng tốt nhất về điều kiện để vẽ trước
  một glyph. Nó chạy trên một thread EDT duy nhất.
- **DelegatedInkTrail của Chromium:** app công bố metadata (màu, đường kính, điểm cuối đã vẽ); viz dự đoán và vẽ nét
  vượt quá điểm đó, bỏ điểm cũ hơn metadata của frame sau
  (`references/chromium/components/viz/service/display/delegated_ink_trail_data.h`) [V]. Đây là mẫu "công thức cộng xác
  nhận theo timestamp".
- **Front buffer:** `GLFrontBufferedRenderer` của Android ghép một layer front-buffer với một layer nhiều buffer, Google
  khuyên chỉ dùng cho vùng nhỏ vì có thể xé hình [V]. Đường canvas 2D độ trễ thấp của Chromium có cả nhánh Android
  (SurfaceControl, sau feature flag), không chỉ Windows và ChromeOS [V, Codex sửa].
- **Dự đoán:** `MotionPredictor` của Android tự rút ngắn khoảng dự đoán khi độ tin thấp
  (`references/android-frameworks-base/core/java/android/view/MotionPredictor.java`) [V]; "Input Framer" của Chrome chờ
  input muộn tới ⅓ vsync [V].
- **Tương tự từ nơi khác** [I]: timewarp bất đồng bộ và late latching của VR; rollback GGPO; Outatime (MobiSys '15) vẽ
  trước nhiều tương lai.
- **Phản hồi widget ngoài UI thread đã có** [V, Codex; bản trước ghi "chưa thấy ai làm" là sai]: Chromium xử lý click và
  kéo thumb scrollbar trên compositor thread (`references/chromium/cc/input/scrollbar_controller.h:24`); Windows
  Composition có animation theo input, hit-test, chạy ngoài thread
  ([MS](https://learn.microsoft.com/en-us/windows/apps/develop/composition/input-driven-animations)); ripple khi nhấn của
  Android chạy trên RenderThread (hẹp hơn: UI khởi động, RenderThread chạy animation).
- **Chạy thử handler thật trong nhánh suy đoán đã có** [V, Codex]: **Crom** (Microsoft Research) chạy handler JavaScript
  có sẵn trong một ngữ cảnh trình duyệt nhân bản và commit kết quả khi sự kiện khớp
  ([Crom](https://www.microsoft.com/en-us/research/publication/crom-faster-web-browsing-using-speculative-execution/));
  một patent GUI dự đoán 2005 chuẩn bị việc widget lúc rảnh và bỏ khi lệch; VNC suy đoán (USENIX '08).
- Zed, VS Code, Sublime, Xi vẽ ký tự sau khi model cập nhật: **không kiểm được** [Codex], đã bỏ khẳng định này.

### 3.3 Vì sao chưa ai làm trọn

Compositor của trình duyệt và hệ điều hành không biết ngữ nghĩa của nội dung, nên chỉ làm được trường hợp hẹp đã khai
báo trước (scrollbar, nét bút, animation khai báo). Toolkit native một thread như IntelliJ biết ngữ nghĩa nhưng không có
render thread. Crom suy đoán ở mức trang web, không ở mức toolkit có render thread.

**Phạm vi của "đầu tiên"** (thu hẹp sau kiểm chứng): điều đáng thử [I] là một toolkit native **sinh delta paint từ việc
chạy component trong nhánh rẽ**, rồi render thread áp nó trước commit chính thức, cho các widget không khai báo trước.
Đó là giả thuyết cần thí nghiệm 3.7 chứng minh, không phải điều đã biết là mới.

### 3.4 Trạng thái nào vẽ trước được

| Chuyển trạng thái | Render thread cần biết trước | Xác nhận | Hỏng khi |
|---|---|---|---|
| Nhấn, hover | Entry hit (đã có trong `Compositing.hits`) và chunk hoặc delta property của trạng thái nhấn | Commit có `consumed ≥ seq` | Handler mở modal hoặc disable node: sai một frame |
| Toggle, checkbox, radio | Chunk cho `!checked` | Như trên | `Model` có `on_change` từ chối |
| Thumb slider, kéo phần tử | Node transform, ánh xạ giá trị → vị trí, kẹp và bước | Commit mang giá trị | Callback snap, giá trị bị app ràng buộc |
| Caret ←/→ | Vị trí x của caret trên dòng (theo grapheme) | Như trên | Bidi, IME, selection |
| Gõ ký tự, ô một dòng | Bảng khoảng 95 ký tự ASCII in được: glyph id và advance, mỗi mục **shape text+c thật** nên kerning và ligature chính xác | Commit mang text | IME, bidi, mask, maxlength, tràn, autocomplete |
| Kéo selection | Các vị trí x của glyph, style của selection | Như trên | Chọn theo từ hoặc dòng |
| Cuộn, fling | Đã có | Đã có | Đã có |

**Front buffer qua wgpu không làm được an toàn:** DXGI flip cần ít nhất 2 buffer, CAMetalLayer không single-buffer,
wgpu không có surface một buffer [V/I]. Overlay cho caret hoặc nét bút cần NSView, HWND con hoặc subsurface, tức handle
thô và `unsafe`.

### 3.5 Thiết kế đề xuất

**Điều chỉ ta làm được:** UI thread sinh công thức bằng cách **chạy chính code component trên input giả định trong một
frame rẽ nhánh rồi diff kết quả** (`Commit::between`). Công thức đến từ logic thật của app, không phải đoán ngữ nghĩa.
`use_model` đã biết instance có bị điều khiển từ ngoài không (ADR-0025).

1. **Định tuyến input (ADR-0015):** platform thread gửi sự kiện con trỏ và phím, mỗi sự kiện có `seq` tăng dần, cho cả
   render thread lẫn UI thread.
2. **ADR mới "Công thức vẽ trước":** commit có thể mang `recipes: Arc<[Recipe]>`, khoá `(NodeKey, base_commit,
   trigger)`. Trigger: nhấn hoặc nhả trong vùng hit, vào hoặc ra, phím hoặc ký tự ở ô đang focus, trục kéo. Nội dung:
   chunk thay thế và delta property, hoặc bảng glyph với quy tắc dời caret.
3. Công thức dựng lúc rảnh, trong frame rẽ nhánh, chỉ cho node dưới con trỏ, node đang focus và phím kế có khả năng.
4. **Hộp cát:** nhánh nào chạm ra ngoài engine (gửi mailbox, clipboard, timer thoát, `on_change` của model bị điều
   khiển mà không khai báo thuần) thì bị huỷ, node đó không có công thức. Chỉ luật của gui/primitives và `Model` gắn
   thẳng được coi là thuần.
5. **Render thread khi có input:** hit-test trên `Presented`; chỉ áp công thức nếu `base_commit` đúng là commit đang
   hiện và mọi input trước chưa xử lý đều đã được vẽ trước hoặc vô hại. Kết quả nằm trong một lớp phủ trên scene đã
   commit.
6. **Ngưỡng cắt muộn:** xem lại input ngay trước khi encode; công thức chỉ đổi property thì ghi qua `queue.write_buffer`
   sát lúc submit (late latching).
7. **Xác nhận:** commit mang `consumed: seq`; lớp phủ của mọi `seq ≤ consumed` bị bỏ, commit là chuẩn. Nếu lớp phủ khác
   commit ở các node đó thì ghi một lần đoán sai (bộ đếm và trace devtools); trong test là fail.
8. **Glyph theo điều kiện của IntelliJ:** cuối run, LTR, không preedit IME, không selection, một dòng, không mask, còn
   chỗ. Chỉ vẽ trước tối đa một ký tự vượt commit cuối, trừ khi font không có GPOS/GSUB theo ngữ cảnh [I].
9. Không có commit xác nhận thì lớp phủ giữ nguyên tới khi commit thay; nó phản ánh hành động của chính người dùng nên
   không rollback theo thời gian.
10. **ADR-0023:** bật hay tắt vẽ trước thành một trục replay.
11. **ADR-0013:** frame rẽ nhánh lên bản release (mục 2).

### 3.6 Đo trung thực

- Chụp màn hình (Typometer, ScreenCaptureKit) đo frame đã composite, không phải photon; trung vị của hai probe Zed lệch
  nhau tới 6,7 ms, và hai lần chạy của cùng probe polling lệch 10,4 ms [V, Codex sửa]; không probe nào đo photon. Chuẩn là vi điều khiển giả bàn phím USB cộng photodiode trên màn hình, dùng để hiệu chỉnh probe phần mềm
  một lần mỗi máy [I].
- **CI mỗi PR (tất định):** compositor `Headless`, vsync giả, log input replay (`compositor/replay.rs`), chi phí UI giả
  lập. Kiểm: số vblank từ input tới phản hồi bằng 1 cho mọi loại vẽ trước được khi chi phí UI tới một interval; chunk
  của node vẽ trước bằng chunk trong commit xác nhận; 0 lần đoán sai trên bộ log; scene lúc nghỉ giống hệt khi bật và
  tắt.
- **Linux CI (pipeline thật):** Weston hoặc sway headless trên lavapipe, input qua virtual-keyboard, timestamp
  `wp_presentation` trừ dấu thời gian input.
- **macOS (lab):** `CGEventPostToPid` cộng ScreenCaptureKit như zed#66954, hiệu chỉnh bằng photodiode.

### 3.7 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm:** trạng thái nhấn và checkbox trong replay `Headless`, chi phí UI giả lập 0–12 ms ở 120 Hz. Đo số vblank
từ input tới phản hồi khi có và không có công thức, số lần đoán sai, chi phí mỗi nhánh. Sau đó chạy harness của
zed#66954 trên docs để có số ms thật trên macOS.

**Rủi ro:**
- Rust không chứng minh được handler không có tác dụng phụ; hộp cát chỉ bắt được thứ engine thấy.
- Rẽ nhánh trên cây lớn có thể giành thời gian UI của frame thật.
- Input bị điều khiển có kiểm tra hợp lệ sẽ nháy sai một frame khi đoán sai.
- IME và bidi bị loại hoàn toàn.
- Trên macOS, độ trễ trong cửa sổ có sàn do WindowServer; độ lớn chưa đo được (bản trước ghi "2 refresh" là không có
  nguồn [Codex]). Phản hồi ở vblank kế là mục tiêu phụ thuộc deadline, không phải bảo đảm.
- Front buffer và surface theo layer cần `unsafe` hoặc việc ở upstream.
- wgpu #9019: acquire có thể không bị chặn với FIFO, làm sai pha vsync mà ngưỡng cắt dựa vào.

---

## 4. Chạy lại tất định toàn bộ app

**Mục tiêu.** Mọi bug report là một file chạy lại được từng bit tới cùng các frame, kéo được tới bất kỳ frame nào, tự
bisect qua commit, tự thu nhỏ, và so được hai lần chạy. Đây là thuộc tính của mọi app shadcn-rs, và là nền để AI sửa
lỗi trên bằng chứng.

### 4.1 Nguồn bất định trong code của ta

| Nguồn | Ở đâu | Trạng thái | Cách đóng |
|---|---|---|---|
| Input nền tảng | `compositor/input_log.rs` (định dạng văn bản v1, `GUI_RECORD_INPUT`) | Đã ghi: con trỏ, wheel, phím, IME, touch, `Blur` [V] | Thêm `Resized`, đổi scale, `Focused`, `Occluded`, `ThemeChanged`, thả file; `window.rs` xử lý focus, theme, occluded, resize, scale nhưng `Event` chưa mang được; thả file thì `window.rs` chưa xử lý [V] |
| Thời gian frame, timer | `clock.rs` | Script điều khiển được; timer chạy theo thời gian script [V] | Ghi thời điểm present của mỗi frame; timer suy ra từ đồng hồ |
| **Deadline của lane idle** | `component.rs`: `DEADLINE = Instant::now() + IDLE_BUDGET`, render idle sau đó bị hoãn | **Chưa kiểm soát: thời gian thực quyết định component nào render** [V] | Ghi số lần resolve idle trước điểm cắt mỗi frame, hoặc tính ngân sách bằng đơn vị việc thay vì thời gian |
| Thời điểm mailbox giao | `mailbox.rs::deliver` | Thứ tự trong một hộp giữ nguyên; rơi vào frame nào tuỳ thread [V] | Ghi mỗi lần giao: (frame, chỉ số hộp theo thứ tự tạo, số lượng, payload hoặc không) |
| Việc ngoài thread | `list/view.rs` (sort/filter trên `std::thread::spawn`), `canvas/planner.rs` | Headless GPU chờ planner canvas có timeout (`gpu/world.rs:509`); `View::spawn` về qua mailbox, không có rào headless nào [V]. Ngoài ra hai lần sort khác nhau trên cùng snapshot có thể về sai thứ tự, vì `View::version` chỉ định danh snapshot (`list/view.rs:52-69,88-91`) [V, Codex] | Việc thuần (sort của `View`) chạy lại tại chỗ ở điểm giao đã ghi; việc không thuần (mạng) ghi payload |
| Pool layout | `layout/engine/mod.rs` | Code khẳng định kết quả giống hệt bất kể số thread và thứ tự job [V là có khẳng định] | Kiểm trong replay: chạy với 1 và 8 thread, so digest |
| Thứ tự duyệt hash | 30 file trong `crates/gui/src` dùng `HashMap`; foldhash có seed theo process và theo hasher; `access.rs` dùng seed cố định cho định danh node [V] | Chưa thấy chỗ nào lộ thứ tự ra output: hai chỗ `.values()` đã nêu chỉ cộng tổng hoặc kiểm va chạm ở debug [V, Codex]. Rủi ro là tương lai, không phải hiện tại | Seed cố định khắp gui (khoá là của ta, không phải của kẻ tấn công), cộng một luật clippy |
| Địa chỉ làm khoá | `style.rs` băm và so địa chỉ `Arc` (ba chỗ); `gpu/mod.rs` | **Không** trúng nhầm: tra style upgrade một weak child, child sống giữ strong cha nên địa chỉ không tái dùng được; texture ảnh giữ clone của nguồn [V, Codex; bản trước gọi đây là lỗi đúng sai là sai] | Chỉ không dùng được cho digest xuyên lần chạy: băm theo id ổn định thay vì địa chỉ |
| Font | `text/fonts.rs`: `system_fonts: true` | Chưa ghi | Ghi digest của bộ font; replay khác máy dùng chế độ chỉ font đi kèm |
| Cài đặt chữ, giảm chuyển động | `text/settings.rs`, `environment.rs` | Đọc trực tiếp, chưa ghi | Ghi vào header và ghi mỗi lần đổi |
| Ngày, múi giờ | `jiff` trong calendar | Chưa ghi | Đọc qua `gui::clock` và ghi |
| Clipboard | `clipboard.rs`: `arboard::paste` | Chưa ghi | Ghi kết quả mỗi lần `paste()` |
| libm giữa các hệ điều hành | 33 lời gọi `powf`/`exp`/`sin`/`cos`/`atan2`/`ln` trong gui, tính cả test [V, Codex; bản trước đếm dòng] | Giống nhau trên một máy, khác giữa các OS [I]; Factorio từng lệch replay ngay khi đổi OS ([FFF #36](https://www.factorio.com/blog/post/fff-36)) | Dùng `libm` viết bằng Rust trong vùng tất định; 37 lời gọi `mul_add` làm tròn đúng trên mọi CPU [I] |
| Output GPU | `path_compute.wgsl`: slot tile lấy bằng `atomicAdd` rồi sắp theo x | Tổng backdrop là atomic số nguyên nên không phụ thuộc thứ tự; x bằng nhau có thể đổi thứ tự cộng float [I] | So scene thay vì pixel (4.3) |

**Nền móng tốt nhưng chưa đủ:** TLS của UI thread nằm trong các khai báo `engine_local!` có nhãn, arena reactive cùng
cây instance có undo log. Nhưng state toàn process và state dùng chung nằm ngoài registry (mục 2), nên "mọi thứ replay
phải tái tạo đã được liệt kê" chưa đúng; phải khép phần đó trước.

### 4.2 Tiền lệ và giới hạn của họ

- **rr:** ghi ở mức syscall, ép mọi thứ chạy trên một core, chậm dưới 2 lần trừ `make`
  ([ATC'17](https://arxiv.org/pdf/1705.05937)) [V]; không theo được lời gọi driver GPU [I]. **Pernosco** dựng cơ sở dữ
  liệu toàn tri trên trace của rr.
- **Microsoft TTD:** ghi ở mức lệnh, chậm 10–20 lần, 5–50 MB/s trace
  ([MS](https://learn.microsoft.com/en-us/archive/blogs/windbg/time-travel-debugging-ttd-file-size)) [V].
- **Replay.io:** chặn khoảng 400 lời gọi libc; "tất định hiệu quả" phủ state JS và DOM quan sát được, cho phép GC và JIT
  khác (tắt một số tối ưu JIT cụ thể, xếp thứ tự một số tài nguyên đồng bộ); vấn đề còn lại là IPC và duyệt hash phụ thuộc
  con trỏ ([blog](https://www.replay.io/blog/effective-determinism)) [V, Codex]. Con số "phí 3–10%" bản trước không kiểm
  được, đã bỏ.
- **Factorio:** CRC của cả bản đồ và save gắn nhãn mỗi tick **trong một chế độ chẩn đoán riêng**, tìm điểm lệch bằng so save
  ([FFF #29](https://direct.factorio.com/blog/post/fff-29), [#47](https://www.factorio.com/blog/post/fff-47)) [V].
  **Overwatch** đồng bộ state, không lockstep tất định; replay dùng lại serializer mạng để tạo snapshot đầy đủ định kỳ cộng
  delta ([GDC](https://gdcvault.com/play/1024053/Replay-Technology-in-Overwatch-Kill)) [V].
- **Kiểm thử mô phỏng tất định:** FoundationDB mô phỏng cả cluster trong một process với scheduler tất định; VOPR của
  TigerBeetle chạy vòng mô phỏng có seed, tái hiện lỗi từ seed cộng commit; Antithesis chạy mỗi VM trên một core vật lý,
  nhiều VM song song
  ([tổng quan](https://newsletter.pragmaticengineer.com/p/antithesis)) [V]; madsim, turmoil, loom, shuttle cho Rust [I].
- **WebRender** lưu display list và tài nguyên của một frame (`capture.rs`) và chạy lại bằng `wrench` [V]: tiền lệ gần
  nhất của việc dùng scene làm trọng tài.
- **Elm, Redux DevTools:** chạy lại reducer, không chạy lại tác dụng phụ [I].

**Vì sao toolkit GUI chưa có:** chúng mượn event loop của hệ điều hành hoặc trình duyệt, để thread chạy tự do, đọc đồng
hồ ở bất kỳ đâu, khoá cache bằng con trỏ, link thư viện C, coi pixel là chân lý, không liệt kê được state của chính
mình, và gắn bản ghi với một binary. Mọi hệ ở trên mua tính tất định từ bên dưới (syscall, hypervisor, trình duyệt) với
giá 1,1–20 lần. Sở hữu cả chồng cho phép ta đặt ranh giới ở engine.

### 4.3 GPU không tất định giữa các máy

Wrench của WebRender có 23 file `reftest.list`, 19 trong đó chứa 313 chú thích (248 `fuzzy(`, 65 `fuzzy-if(`; thêm 7
`fuzzy-range(` là 320) [V, Codex]; một file test GPU của Chromium dùng matcher fuzzy và Sobel của Skia Gold với
`max_different_pixels` từ 130 tới 31 100 theo từng test (không phải giới hạn chung) [V]; tester web của Flutter mặc định
so mờ 3×3 cho blur và khử răng cưa (nguồn không nói nguyên nhân là Chromium) [V, Codex sửa]. Hàm builtin của WGSL có biên sai số, driver có thể đổi thứ tự phép tính [I].

**Kết luận:** replay so chính xác scene phía CPU (chunk và display list), hộp layout, cây a11y và các lần ghi signal.
Pixel chỉ là kiểm tra tuỳ chọn, chỉ hợp lệ trên cùng adapter.

### 4.4 Thu nhỏ, bisect, riêng tư

- **Bản ghi của người:** delta debugging (ddmin, Zeller và Hildebrandt, TSE 2002) trên **cử chỉ nguyên vẹn** (nhấn…nhả,
  preedit…commit, wheel bắt đầu…kết thúc) để mọi log thu nhỏ vẫn hợp lệ.
- **Lỗi của monkey và seed:** thu nhỏ các lần rút từ `Rng` thay vì thu nhỏ sự kiện, như Hypothesis (MacIver, ECOOP 2020),
  để mọi input thu nhỏ vẫn hợp lệ [I].
- **Bisect, không tương thích ngược** (người dùng chốt 2026-11-23: không có ngoại lệ). Định dạng `.rec` đi theo code,
  không có phiên bản cũ nào được đọc. Commit nào đổi định dạng thì chuyển luôn mọi bản ghi trong `tests/recordings/`
  sang định dạng mới trong cùng commit, bằng lệnh `replay migrate`, và xoá bộ đọc cũ: ta chuyển dữ liệu, không giữ
  shim. Bisect một bản ghi qua các commit dùng bản `replay` của commit đầu khoảng (tức là bản đã ghi) để dịch bản ghi
  sang lớp sự kiện thô của winit tại mỗi bước. Lớp đó là ngôn ngữ của nền tảng, không phải API của ta, nên ít đổi;
  commit không nhận được thì `git bisect` coi là "skip". Mỗi sự kiện ghi kèm đích ngữ nghĩa (`@NodePath`, nhãn a11y) để
  replay phát hiện khi đích đã dời.
- **Riêng tư:** SDK trình duyệt của Sentry che chữ và chặn phần tử media ở phía client mặc định; phát lại dựng từ DOM và
  sự kiện, không chạy lại logic app ([Sentry](https://docs.sentry.io/platforms/javascript/session-replay/privacy/)) [V].
  Castro, Costa và Martin đã sinh input thay thế riêng tư mà vẫn giữ đường lỗi
  ([MSR](https://www.microsoft.com/en-us/research/publication/better-bug-reporting-with-better-privacy/)) [V, Codex bổ
  sung]. Thứ tự đề xuất: thu nhỏ trước (bỏ phần lớn chữ đã gõ), rồi thay ký tự bằng ký tự cùng loại (chữ → `a`,
  số → `0`, giữ độ dài và loại grapheme) trong ô đánh dấu nhạy cảm, paste và payload; rồi **chạy lại bản đã che**: nếu
  lỗi vẫn tái hiện và digest khớp tới điểm lỗi thì bản ghi được đánh dấu "đã che và đã kiểm". Điều này chỉ chứng minh bản ghi đó tái hiện được, không chứng minh
  hành vi không đổi ở chỗ khác.

### 4.5 Thiết kế đề xuất

1. **Một file ghi `gui-record 1`.** Header: commit, nền tảng, adapter, digest font, cài đặt chữ, locale và múi giờ,
   ngày, kích thước và scale, giảm chuyển động, seed, `GUI_LAYOUT_THREADS`. Thân: một dòng có thứ tự gồm sự kiện nền
   tảng, thời điểm present của frame, lần giao mailbox, lần đọc clipboard, số resolve idle trước điểm cắt, lần đổi cài
   đặt. Mở rộng từ định dạng `input_log.rs` hiện có.
2. **Ranh giới:** UI thread cộng pool layout là hàm thuần của dòng đó; không gì khác đi vào. Luật clippy
   `disallowed-methods` cấm `Instant::now` và tương tự bên trong ranh giới, trừ `clock.rs` và một module `measure`
   (`app.rs` và `window.rs` hiện gọi 16 lần).
3. **Thread:** layout vẫn song song, replay kiểm kết quả khớp giữa các số thread. Việc thuần (`View::spawn`, planner
   raster) chạy lại tại chỗ ở điểm giao đã ghi; việc không thuần ghi payload (`T: Record`). Render thread đã replay được
   trên vsync giả (`replay.rs`).
4. **Digest mỗi frame** như CRC từng tick của Factorio: băm chỉ các chunk ghi lại trong frame, diff layout, thay đổi a11y
   và lần ghi signal. Phí khoảng một lần băm mỗi chunk đổi, dưới 1% [I].
5. **Hash và khoá:** seed foldhash cố định khắp gui; digest băm theo id ổn định, không theo địa chỉ; state toàn process (mục 2) vào registry; `libm` bằng Rust cho
   replay khác hệ điều hành.
6. **Kéo tới frame:** chạy lại từ đầu (frame headless tốn vài ms, nên 18 nghìn frame khoảng một phút [I]); giữ các
   process dừng ở mốc đều để kéo nhanh.
7. **Lùi từng bước** bằng undo log Fork có sẵn cho W frame gần nhất. Lùi state đồ hoạ sống đã có tiền lệ: CoExist/Lively
   Kernel, tua ngược trong engine game Braid, ReJS cho web app [V, Codex]; điểm khác của ta chỉ là dùng undo log có sẵn
   của engine UI cộng digest scene.
8. **`replay diff a b`:** căn hai lần chạy theo chỉ số input, tìm frame đầu tiên digest khác, báo phase khác, lần ghi
   signal, `LayoutDiff`, item của scene và node a11y.
9. **Lệnh devtools (ADR-0010):** `open rec`, `seek N`, `step`/`back`, `first-divergence`, `why @node` (lần ghi nào gây
   ra), `minimize`, `bisect`, `redact`. Đều là văn bản thuần, agent dùng được.
10. **CI:** mọi lỗi stress hoặc monkey ghi một file `.rec`, tự thu nhỏ và tải lên; `tests/recordings/` chạy lại trong
    `cargo test` so với digest kỳ vọng nằm ở file riêng, cập nhật được như insta.
11. **ADR đổi:** 0015 (log thành bản ghi), 0023 (scheduler và mailbox), 0010 (lệnh mới), 0011/0017 (scene là trọng tài;
    atomic của path), 0019 (font), 0014 (khẳng định giống hệt từng bit được kiểm), và một ADR mới.
12. **Kích thước:** khoảng 25 byte mỗi sự kiện ở 120 Hz, tức khoảng 180 KB mỗi phút trước nén [I], so với 5–50 MB/s của
    TTD.

### 4.6 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm:** monkey 500 bước trên scene stress `bigtable`. Ghi dòng mở rộng (lần giao, thời điểm frame, điểm cắt
idle). Replay hai lần headless ở hai process với seed foldhash khác nhau, một lần `GUI_LAYOUT_THREADS=1`, một lần `=8`.
Báo frame đầu tiên digest khác, số byte mỗi phút, phí ghi theo số lệnh mỗi frame, tốc độ replay. Riêng: vẽ một scene 100
lần trên Metal và trên lavapipe, băm pixel đọc lại, để kiểm mối lo về atomic của path.

**Rủi ro:**
- Deadline của lane idle là lỗ hổng thật hôm nay.
- Crate bên thứ ba (parley, fontique, arboard) có thể giấu bảng hash hoặc đọc đồng hồ.
- Mọi kiểu payload của mailbox cần được đánh dấu để ghi.
- Không có định dạng ổn định, nên bisect qua một lần đổi lớn của lớp sự kiện thô sẽ phải "skip" các commit đó.
- Kéo tới frame bằng chạy lại từ đầu sẽ chậm với phiên dài.
- Che dữ liệu có thể đổi hành vi; bước chạy lại để kiểm chỉ phát hiện, không ngăn được.
- Golden pixel GPU vẫn phải so mờ.

---

## 5. App mà AI agent vận hành được

**Mục tiêu.** Mọi app viết bằng shadcn-rs tự có, ngay từ cách dựng, một giao diện có kiểu, ổn định và có ngữ nghĩa cho
agent: đọc state, thực hiện hành động ngữ nghĩa, xem trước hậu quả an toàn. Cùng nền đó làm accessibility tốt hơn ARIA,
kể cả cho chart và canvas.

### 5.1 Hiện trạng của ngành

**Tỉ lệ thành công.**
- **OSWorld:** lúc ra mắt agent tốt nhất đạt 12,24% so với người 72,36%. Trên OSWorld-Verified, Agent S3 đạt 72,58% với
  10 rollout (12/2025), vượt con số người đo trên bộ *gốc*; cấu hình một rollout 65,16–67,46%. Bốn mục đầu nay 83,39–90,19%;
  16 trên 144 mục có điểm vượt con số người. Các mục khác cấu hình nên con số không tự chứng minh bão hoà. OSWorld 2.0
  (6/2026): 20,6% strict ở bài ra mắt, 44,44% trên bộ đủ v2.1 [V, Codex,
  [dữ liệu Verified](https://osworld-v1.xlang.ai/static/data/osworld_verified_results.xlsx),
  [OSWorld 2.0](https://arxiv.org/html/2606.29537)].
- **Grounding (ScreenSpot-Pro):** 0,9% (GPT-4o) và 18,9% (OS-Atlas-7B) lúc ra mắt; GroundAnything 75,96%, và cùng bài báo
  ghi GPT-6 Astra 93,17% ([GroundAnything](https://arxiv.org/html/2609.39600)) [V, Codex].
- **WebArena:** 14,4% (GPT-4) so với người 78%; CUA của OpenAI sau đó 58,1%. VisualWebArena: 7,25% chỉ từ chữ, 15,05% với
  GPT-4V [V, thứ cấp].

**Các lớp lỗi.**
- **Nguyên nhân gốc** (CUADebug, mỗi quỹ đạo hỏng gán một nguyên nhân): grounding 25/204 (khoảng 12%), suy luận và điều
  khiển 110/204 (khoảng 54%), nhận thức 36/204 (khoảng 18%) ([CUADebug](https://arxiv.org/html/2608.02643)) [V, Codex].
  Bản trước ghi thêm "20–61% khi nhiều nhãn" và "phần lớn là thiếu state": nguồn **không** chứng minh hai điều đó [Codex].
  Việc thiếu state (cái gì đang chọn, ngoài màn hình, vừa đổi) là giả thuyết thiết kế của ta [I], thí nghiệm 5.6 phải
  kiểm.
- **Phản hồi chậm:** agent mất hàng chục phút cho việc người làm trong khoảng 2 phút, dùng 1,4–2,7 lần số bước cần thiết;
  lập kế hoạch và tự soát chiếm 75–94% độ trễ ([OSWorld-Human v1](https://arxiv.org/html/2506.16042v1)). Con số 66% bước
  vòng lặp và 54% hồi phục ở bản v2, chỉ trên các lần chạy hỏng quá 50 bước ([v2](https://arxiv.org/html/2506.16042v2))
  [V, Codex].
- **Token:** ước lượng ảnh chụp 1280×800 khoảng 1.365 token; trên sáu trang thử, cây accessibility cả trang tốn 2,8–14,5
  lần một ảnh viewport; cây chỉ viewport 229–1.516 token [V, Codex, nguồn blog].

**Các cách dùng giao diện có cấu trúc, và thứ mỗi cách phải chấp nhận.**
- **Aria snapshot của Playwright, browser-use, UIA cho UFO2, AX của macOS:** tên đoán bằng heuristic, giá trị là chuỗi,
  không có dữ liệu ngoài màn hình, tham chiếu chỉ sống trong một snapshot [I].
- **WebMCP** (`document.modelContext.registerTool`): bản nháp W3C Community Group, origin trial Chrome 149–156. Tool viết
  tay cạnh UI nên có thể lệch khỏi UI [V]. Handler của trang **đọc được** state của app, và bản nháp có cả tool form khai
  báo [V, Codex; bản trước ghi "không thấy state" là sai].
- **Android AppFunctions:** AndroidX AppFunctions 1.0.0 **stable từ 18/11/2026**; gọi chéo package cần quyền đặc quyền,
  app tự gọi hàm của mình thì không; Google có riêng đường tự động hoá UI cho một số app
  ([release notes](https://developer.android.com/jetpack/androidx/releases/appfunctions)) [V, Codex; bản trước ghi
  "alpha/beta" là cũ].
- **Apple App Intents:** `requestConfirmation`; iOS 27 có metadata rủi ro kế thừa từ schema [V, thứ cấp].
- **Windows:** connector của agent là MCP server sau một registry trên máy, có proxy kiểm toán lưu lượng; agent chạy
  trong "Agent workspace" riêng dưới tài khoản riêng
  ([Microsoft](https://support.microsoft.com/en-us/windows/ai/ai-features/experimental-agentic-features)) [V].

**Giới hạn chung:** tất cả là kênh phụ. Hoặc API ý định và UI là hai chương trình (WebMCP, AppFunctions, App Intents),
hoặc ngữ nghĩa bị đoán ngược từ UI (cây accessibility, pixel). Không bên nào xem trước được hậu quả, vì không bên nào
sở hữu state của app.

### 5.2 Giới hạn của accessibility, hại cả người lẫn agent

- **Chart:** người dùng trình đọc màn hình trích thông tin kém chính xác hơn 61,48% và tốn thêm 210,96% thời gian (87% so
  với 34% đúng), kể cả khi có alt text hoặc bảng dữ liệu
  ([Sharif và cộng sự, ASSETS'21](https://dl.acm.org/doi/fullHtml/10.1145/3441852.3471202)) [V].
- **Mô hình của AccessKit** (`references/accesskit/accesskit/src/lib.rs`) [V]: 182 role, 22 action; giá trị chỉ là
  `Value(Box<str>)` hoặc `NumericValue(f64)`, không có kiểu ngày, enum hay khoảng; `CustomAction` là `{id: i32,
  description}` không tham số; có chỉ số dòng và cột nhưng không truy vấn được dòng chưa dựng; không có gì cho series của
  chart, không đánh dấu hành động phá huỷ, hành động không phải transaction.
- **Của ta:** `chart_summary` (`crates/shadcn/src/chart.rs:147-166`) đưa ra nhãn dòng, nhãn series và giá trị đã định
  dạng dưới dạng chữ, bật theo lựa chọn; donut của docs dùng nó [V, Codex; bản trước ghi "không đưa dữ liệu series" là
  sai]. Cái thiếu là cây có cấu trúc, điều hướng được (series → điểm) và tự động cho `chart_xy`.

### 5.3 An toàn của hành động agent

- **Prompt injection:** Claude for Chrome 23,6% tấn công thành công khi chưa giảm thiểu, 11,2% khi có (pilot); khoảng 1%
  là đánh giá sau với biện pháp mới, tuỳ model; NIST CAISI: tấn công thích nghi mạnh nhất 11% → 81% trên tác vụ
  Workspace giữ lại [V, Codex; "90%+" không kiểm được]; AgentDojo: giới hạn tool trước khi agent thấy dữ liệu không tin cậy giảm
  tấn công có mục tiêu từ 57,7% xuống 6,8%; hướng ở mức hệ thống là kiểm soát luồng thông tin
  ([SPIDES](https://arxiv.org/pdf/2610.24277)) [V, thứ cấp].
- **Hệ hiện có làm gì:** sandbox VM và allowlist, xác nhận (App Intents), proxy kiểm toán và tài khoản riêng (Windows),
  gợi ý `destructiveHint`/`readOnlyHint` của MCP [I].
- **Chưa ai có:** chạy thử một hành động GUI, undo xuyên ranh giới agent, nguồn gốc theo từng lần ghi.

### 5.4 Danh tính ổn định

Selector web (CSS, XPath, ref aria) gãy khi render lại, đổi thứ tự, đổi ngôn ngữ, ảo hoá. Ta có id AccessKit từ băm
`NodePath` có kiểm va chạm ở debug (ADR-0021), instance theo vị trí + key + kiểu, `primitives::collection::Items` theo
chỉ số dữ liệu (ADR-0025) [V]. Đường dẫn theo vị trí vẫn dời khi sibling đổi; selector agent tin được cần id ngữ nghĩa:
kiểu component, đường key, và key dữ liệu của item [I].

### 5.5 Thiết kế đề xuất

- **Ngữ nghĩa từ cách dựng:** một phase `Semantics` thay phase a11y của ADR-0021. Mỗi primitive (Select, Slider,
  DatePicker, Listbox, Tree, Dialog, Toggle…) khai báo role; giá trị có kiểu (`Date`, `Range<T>`, `OneOf<enum>`,
  `Set<Key>`, `Text`, `Number{min,max,step}`) kèm schema; các `Intent` có tham số kiểu, điều kiện trước và lớp rủi ro;
  quan hệ (nhãn, controls, owns, `activedescendant`). AccessKit là một hình chiếu mất thông tin; schema cho agent là hình
  chiếu không mất. Vì primitives đã giữ hành vi (ADR-0025), mọi component shadcn thừa hưởng không tốn công tác giả.
- **Intent chạy qua đúng đường của người dùng:** intent biên dịch thành handler của chính primitive (`Slider::set`,
  `Select::choose(key)`, `Dialog::confirm`), cùng hàm mà keymap và con trỏ gọi, nên hành vi giống hệt theo cấu trúc. Kết
  quả đồng bộ: frame kế đã ổn định, lời gọi trả về diff ngữ nghĩa (giá trị có kiểu, node thêm hoặc bớt, focus, layer mở,
  lần ghi signal).
- **Xem trước bằng rẽ nhánh:** `preview(intent)` chạy trên nhánh (mục 2), trả diff, bỏ nhánh; `commit` chạy thật. Thế
  giới ngoài app (I/O, mạng) phải đi qua outbox khai báo mà nhánh giữ lại và không gửi.
- **Truy cập ở mức dữ liệu:** `query(collection, predicate, fields, range)` chạy trên dữ liệu `Items`, không mount dòng
  nào. Chart công bố series dưới dạng bảng có kiểu (x, y, series, đơn vị, miền) cạnh chunk dữ liệu GPU (ADR-0018). Cùng
  cấu trúc đó cho người dùng trình đọc màn hình một cây kiểu Olli (chart → series → điểm) với tóm tắt tính từ dữ liệu.
- **An toàn bằng khai báo:** lớp rủi ro suy ra từ primitive và variant: nút variant destructive hoặc action của
  AlertDialog là `Destructive`; điều hướng là `Reversible`; gửi hoặc submit là `External`. `Destructive` và `External` cần
  xem trước cộng xác nhận của người, hiện trong một layer agent không với tới (inert và modal của ADR-0024). Quyền theo
  agent và theo lớp intent. **Nguồn gốc:** mọi lần ghi signal mang `Actor::{Human, Agent(id)}` trong trace, cho kiểm toán
  và undo một bước của agent qua cùng undo log.
- **Chặn injection:** chữ được gắn nhãn tại nguồn: `&'static str` tác giả viết là UI, dữ liệu lúc chạy là nội dung.
  Schema cho agent đánh dấu nội dung là dữ liệu, không bao giờ là chỉ dẫn. Intent có tham số chảy từ nội dung sang một
  hành động `External` cần xác nhận, như kiểm soát luồng thông tin nhưng ở mức toolkit.
- **Giao tiếp:** MCP server có sẵn trong mọi app, trên socket cục bộ hiện có (ADR-0010); phát hiện và kết nối cần người
  dùng đồng ý theo từng agent; `--json` cho lệnh devtools; named pipe trên Windows.
- **ADR đổi:** 0021/0009 (phase a11y thành `Semantics`), 0025 (primitive khai báo giá trị, intent, rủi ro), 0010 (giao
  thức JSON/MCP có đồng ý), 0024 (layer quyết định intent nào với tới được), 0013 và engine (rẽ nhánh lên release,
  outbox), 0018 (dữ liệu chart), và ADR mới "App được vận hành qua intent có kiểu".

**Phạm vi của "đầu tiên"** (đã thu hẹp sau kiểm chứng của Codex). Từng mảnh đều có tiền lệ: action có kiểu và metadata
rủi ro từ schema (App Intents, [WWDC26](https://developer.apple.com/videos/play/wwdc2026/347/)), kiểm soát luồng thông
tin chống injection ([CaMeL](https://arxiv.org/abs/2503.18813), [ETAS](https://arxiv.org/abs/2607.17780)), cây chart
điều hướng được ([Olli](https://vis.csail.mit.edu/pubs/olli/)). Phần có thể là mới [I] là tổ hợp: một intent do toolkit
sinh, chạy qua **đúng handler** của người dùng trên state app được cô lập, trả diff có kiểu, với tác dụng phụ bị chặn
trong outbox. **Hai ý bản trước phải bỏ:** vòng đời của chuỗi (`&'static str` hay không) không chứng minh được nguồn gốc
(chuỗi tĩnh có thể chứa nội dung, chuỗi động có thể là UI), và variant của nút không chứng minh được rủi ro thật của hành
động. Nguồn gốc và rủi ro phải được khai báo tường minh và kiểm, không suy ra từ kiểu.

### 5.6 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm:** 40 tác vụ trên app docs và stress qua `Session` headless (tất định, có seed); bộ kiểm của mỗi tác vụ là
một vị từ trên các signal có nhãn, nên kết quả tái hiện được, khác OSWorld chạy trên VM. Ví dụ: "chọn các dòng lỗi trong
bảng 5.000 dòng rồi xoá", "đặt khoảng ngày là tuần sau", "giá trị cao nhất của series B là bao nhiêu", "đặt slider về
37". Ba nhánh cùng một model: ảnh chụp màn hình, văn bản `a11y` hiện có, và prototype giao diện ngữ nghĩa trên 5
primitive (Select, Slider, DatePicker, Listbox/bảng, chart). Đo: tỉ lệ thành công, số bước, token, độ trễ, số lỗi không
đảo ngược được, và tỉ lệ injection thành công với 10 chuỗi độc hại cài trong UI.

**Rủi ro:**
- Rẽ nhánh ở release tốn bộ nhớ và CPU; state không đăng ký `engine_local!` phá cô lập.
- Tác dụng phụ đi vòng outbox sẽ rò khỏi bản xem trước.
- Id ngữ nghĩa vẫn có thể đổi khi cấu trúc đổi mạnh.
- Nhãn nguồn gốc của chuỗi có thể quá thô với nhãn dựng bằng `format!`.
- Người dùng mỏi vì phải xác nhận nhiều.
- MCP và WebMCP còn đang đổi.
- Hình chiếu sang API accessibility của hệ điều hành vẫn mất thông tin, nên người dùng trình đọc màn hình chỉ được phần
  AccessKit mang được, trừ khi ta góp lên upstream.

---

## 6. Tính tăng dần chứng minh được

**Mục tiêu.** Mọi đường tăng dần (reactive, cache layout, paint, compose, kế hoạch GPU, giao thức commit giữa các
thread) được chứng minh hoặc kiểm vét cạn là cho đúng kết quả như tính lại từ đầu, và chỉ chạy lại khi một input đã
đọc thực sự đổi.

### 6.1 Một lỗi tìm thấy khi đọc code

`crates/gui/src/layout/engine/cache.rs:29` `axis_key` đặt kích thước đã biết và không gian khả dụng `Definite` vào cùng
một không gian `u32`: đã biết là `v.to_bits()`, khả dụng là `(-v).to_bits() ^ 0x0040_0000`. Width đã biết `1.5` và width
khả dụng `Definite(-1.0)` cùng ra `0x3FC00000`, nên một entry có thể trả lời thay cho entry kia. Trục height có cờ
`known_height` để tách hai trường hợp, trục width thì không [V, agent tính bằng Python và tôi tính lại tay].

Codex kiểm độc lập và **xác nhận va chạm xảy ra trong lần tra cache**, với điều kiện: width đã biết được đánh dấu
definite (với width chưa biết, `definite[0]` chuẩn hoá thành `true`), cùng run mode, sizing mode, cờ margin, trục yêu
cầu, kích thước cha liên quan và height. Khi đó `Key::answers` chấp nhận và `NodeCache::get` trả output đã lưu mà không so
input đầy đủ (`cache.rs:29-35,89-103,111-133,317-338`). Có đường thật tới width khả dụng âm: đo intrinsic của block trong
Taffy trừ margin ngang không kẹp trước khi probe con (`taffy-0.15/src/compute/block.rs:1045-1055`), trong khi vài đường
flex có kẹp về 0. **Chưa có** vết widget thật nào cho thấy cùng một node nhận cả hai probe va chạm [V, Codex]. Điều đáng ghi
không phải một ca lỗi: một model checker có giới hạn chạy trên mọi mẫu bit f32 tìm lớp lỗi này trong vài giây, còn test
có seed (8 seed × 12 bước) sẽ không bao giờ tìm ra.

### 6.2 Hiện trạng [V]

- **Tập đọc của layout là khai báo, không phải ghi lại.** `Reads` lấy từ `NodeStyle::reads_parent()`, từ
  `Measure::reads()` (mặc định "đọc hết", nên override sai sẽ khai thiếu) và từ `heights_read()`
  (`layout/engine/mod.rs:509-566`), một bản tóm tắt viết tay về những gì thuật toán của Taffy đọc. Đây đúng là lớp lỗi
  của Taffy [#1149](https://github.com/DioxusLabs/taffy/issues/1149) và
  [#1306](https://github.com/DioxusLabs/taffy/issues/1306): bản tóm tắt trôi khỏi thuật toán. `verify()` chỉ kiểm node đã
  đổi.
- **Ba cặp đường đôi viết tay:** `paint.rs` `record_all` / `record_changes`; `compositor/compose.rs` `whole` /
  `compose_changed`; `gpu/frame.rs` `plan` / `replan` (chỉ mục `by_transform` của `replan` là một chỉ mục join viết tay).
- **Cold check** bị chặn bởi `debug_assertions`; `GUI_COLD_CHECK` chỉ là công tắc mặc định. `Scenario::check_cold(true)`
  bật trực tiếp, và test tích hợp 8 seed × 12 bước dùng đúng nó (`crates/gui/tests/seeded.rs:32-38`), nên **CI đã chạy cold
  check** ở test đó [V, Codex; bản trước và review §6.5 ghi "CI không bật" là sai]. Cái thiếu là cold check trên mọi
  Scenario và bước stress, và ở release.
- **Giao thức thread:** `compositor/thread.rs` chờ bằng `recv_timeout` và đọc `Instant::now()`, tức thời gian nằm trong
  vòng lặp. ADR-0015 có 8 bất biến, kèm nhiều test cuộn, 7 test replay input log và số đo stress, nhưng không bất biến nào
  được kiểm trên mọi thứ tự đan xen. Workspace không có loom, shuttle hay proptest [V].

### 6.3 Tiền lệ: ai đã chứng minh gì, ai đã gãy ở đâu

- **Build systems à la carte** ([JFP 2020](https://simon.peytonjones.org/assets/pdfs/build-systems-jfp.pdf)) [V] cho hai
  định nghĩa ta dùng. *Đúng*: mọi khoá không phải input bằng task của nó tính trên store kết quả. *Tối thiểu*: mỗi task
  chạy nhiều nhất một lần mỗi lần build, và chỉ khi nó phụ thuộc bắc cầu vào input đã đổi.
- **Adapton / Nominal Adapton** ([arXiv](https://arxiv.org/pdf/1503.07792v4)): chứng minh nhất quán với tính từ đầu,
  trên giấy, không cơ giới hoá [V].
- **DBSP** ([VLDB 2023](https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf)): toán tử ·Δ biến mọi truy vấn trên luồng thành
  dạng tăng dần; mọi định lý chứng minh trong khoảng 5K dòng Lean vì đại số nhỏ [V]; bài tiếp theo thêm fixpoint trong
  Lean 4 ([arXiv](https://arxiv.org/pdf/2610.00530)) [V]. **Bài học: suy ra đường tăng dần, đừng viết tay.**
- **Salsa** có bug mở đúng các lớp của ta [V]: [#1326](https://github.com/salsa-rs/salsa/issues/1326) (kết quả cũ khi
  input chỉ đọc có điều kiện, tức tập đọc thiếu), [#1310](https://github.com/salsa-rs/salsa/issues/1310) và
  [#834](https://github.com/salsa-rs/salsa/issues/834) (backdate, tức cắt theo bằng nhau, sai),
  [#1411](https://github.com/salsa-rs/salsa/issues/1411) (sổ sách vòng lặp cũ).
- **Spineless Traversal** ([PLDI 2025](https://arxiv.org/pdf/2411.10659)): tốt nhất hiện nay cho thứ tự invalidation của
  layout, trung bình nhanh 1,80×, nhanh hơn ở 83% của 2216 benchmark [V]. Cassius và VizAssert hình thức hoá CSS trong
  SMT để kiểm, không phải cho tính tăng dần [I].
- **Chromium** ship `PaintUnderInvalidationChecking` như một runtime feature: vẽ lại và so display item [V]. Đó là cold
  check sau một cờ lúc chạy, không phải cờ build.
- **ShardStore của S3** ([SOSP 2021](https://jamesbornholt.com/papers/shardstore-sosp21.pdf)) [V]: mô hình tham chiếu
  viết bằng Rust chạy được, kiểm khớp trên chuỗi thao tác ngẫu nhiên, model checking không trạng thái; chặn 16 bug trước
  production; 18% số dòng test harness do người không chuyên sửa lần cuối, và mô hình cộng harness bằng khoảng 20% kích
  thước code thật [V, Codex sửa]. Đây là thứ gần nhất với đường lạnh của ta.
- **Self-adjusting computation** đã có chứng minh cơ giới hoá trong Twelf ([Acar và cộng sự](https://arxiv.org/abs/1106.0478)),
  và build system tiến có chứng minh hình thức ([Forward Build Systems, Formally](https://arxiv.org/abs/2202.05328)) [V,
  Codex bổ sung]. Vậy "chứng minh tính tăng dần" tự nó không mới.
- **Chú thích chính xác hoá** [V, Codex]: Salsa #1326 là timestamp revision cũ khi entry vòng lặp đổi (không phải tập đọc
  thiếu); #834 đã đóng. Spineless Traversal 1,80× là trung bình hình học so với Double Dirty Bit trên 2216 frame của 50
  trang, 17% chậm hơn. Taffy #1149 là thiếu `SizingMode` trong khoá cache, #1306 là kết quả cache bỏ mất baseline đầu:
  hai lỗi khác nhau, cùng họ "khoá thiếu thứ đã đọc". DBSP bài tiếp theo hình thức hoá phát hiện fixpoint và hội tụ
  (11.748 dòng Lean 4 mới).

**Vì sao chưa ai giải:** toolkit nào cũng viết đường tăng dần bằng tay rồi kiểm bằng test ví dụ, vì code của họ có
global, thread tự do và thời gian thực ở khắp nơi. Ta đã có engine liệt kê được TLS của UI thread (`engine_local!`; state
toàn process còn ngoài registry, mục 2), có cấm `thread_local!` thô, có đường lạnh chạy được, nên công cụ hình thức chạm được tới code thật.

### 6.4 Công cụ

| Công cụ | Chứng minh được gì cho ta | Giới hạn |
|---|---|---|
| **Kani** | `cache.rs` (không HashMap, không `dyn`): khoá đúng trên mọi f32, `answers()` khớp vị từ tham chiếu, logic slot second-chance. Danh sách `link`/`unlink`/free list của arena reactive ở ≤4 node, ≤8 link | `HashMap::new` hỏng ([kani#723](https://github.com/model-checking/kani/issues/723)) [V]; closure và `Box<dyn Any>` trong `runtime.rs` nổ không gian trạng thái; vòng lặp cần giới hạn unwind |
| **Verus / Creusot / Prusti / Aeneas / Flux** | Chứng minh trọn vẹn một lõi viết lại | Mỗi cái một toolchain hoặc ghost code; `RefCell`/`Rc` hỗ trợ yếu; quá đắt cho 30K dòng đang đổi [I] |
| **Mô hình Lean 4** | Thuật toán alien-signals trừu tượng (đẩy-kéo PENDING/DIRTY, cắt), như DBSP đã làm | Chứng minh mô hình chứ không phải code, cần test khớp làm cầu nối |
| **loom** | Chỉ các primitive nhỏ | Không mô hình được `recv_timeout` hay thời gian; nổ khi quá vài thread [I] |
| **shuttle / Stateright** | Duyệt ngẫu nhiên (PCT) hoặc vét cạn các thứ tự đan xen thông điệp; Stateright kiểm cả tính chất "rồi sẽ" [I] | Cần vòng lặp là máy trạng thái không I/O bên trong |
| **proptest-state-machine, bolero** | Thu nhỏ; bolero chạy một harness thành fuzzer, property test và chứng minh Kani [I] | |
| **Liệt kê vét cạn ở quy mô nhỏ** (giả thuyết small scope của Jackson) | Mọi DAG ≤5 node × mọi chuỗi ghi ≤3, có đọc có điều kiện và dispose giữa flush | Vài phút CI, không phải hàng giờ |

### 6.5 Tính chất và cách thiết lập

- **Reactive:** nhất quán với tính từ đầu; không glitch (không effect nào thấy phiên bản lẫn lộn); tối thiểu (mỗi effect
  chạy ≤1 lần mỗi flush và chỉ khi giá trị nó đọc đổi sau khi cắt); dispose trả arena về free list đúng; tái dùng link
  không để link cũ. Chứng minh trên mô hình Lean; kiểm vét cạn ở quy mô nhỏ so với oracle tính lại tất cả có đếm số lần
  chạy; Kani cho bất biến danh sách. Test 400 seed hiện tại không sinh phụ thuộc động và không dispose [V].
- **Cache layout:** *đúng* (trúng nghĩa là bằng nhau trên mọi input đã đọc) và *đủ* (không lần đọc nào thoát khỏi việc
  ghi). Ép bằng kiểu: thuật toán nhận `In<'_>` không có field public, mỗi accessor bật một bit đọc. `Reads` được ghi lại
  (như Ladybird), không khai báo; xoá `Measure::reads` và `heights_read`. Phần còn lại là "output là hàm của `In` cộng
  câu trả lời của cây con", Rust bảo đảm được khi code không có global (clippy cấm truy cập `engine_local!` trong layout).
  Kani chứng minh mã khoá đơn ánh trên phần đã đọc.
- **Paint, compose, kế hoạch GPU:** đúng theo cách dựng (6.6). Trong lúc chưa xong: so khác biệt với đường toàn phần
  trong mọi Scenario và mọi bước stress của CI.
- **Giao thức commit:** vòng lặp render thành hàm thuần `step(state, event) -> (state, effects)`, trong đó deadline,
  vsync, panic, mất device đều là event. Duyệt mọi thứ tự đan xen tới độ sâu k. Kiểm: bảo toàn cuộn (delta wheel = delta
  trong token + phần compositor tự cuộn); 8 bất biến của ADR-0015; liveness (mỗi commit được hiện hoặc bị commit sau
  thay); hồi phục (panic ở bất kỳ bước nào rồi một commit đầy đủ bằng một lần dựng lạnh).

### 6.6 Thiết kế đề xuất

1. **Một đường, không phải ba cặp.** Viết `whole` thành pipeline trên các tập (chunk, transform, effect). Suy ra đường
   tăng dần như DBSP: Δ(A⋈B) = ΔA⋈B + A⋈ΔB + ΔA⋈ΔB, và `by_transform` chính là chỉ mục của join đó. `record_all` đã dùng
   lại chunk không đổi, nên paint tăng dần thành `record_all` trên một bảng memo, change log chỉ là gợi ý kiểm (kiểu
   red-green của Salsa). Đúng đắn quy về một bổ đề cho mỗi khoá memo. Xoá `record_changes`, `compose_changed`, `replan`
   [I: tốc độ phải đo so với code viết tay hiện tại].
2. **Tập đọc không thể thoát**, ép bằng kiểu (6.5). Cùng mẫu áp cho render component, vốn đã theo dõi lần đọc qua
   signal.
3. **Cold check lấy mẫu ở release**, sau cờ lúc chạy như Chromium: registry Fork có ở release (mục 2), chạy một frame
   lạnh trên frame rảnh (ví dụ mỗi 60 giây rảnh, ưu tiên thấp). Lệch thì xuất bản ghi replay (mục 4): bản ghi vừa là
   oracle vừa là bug report. Chi phí: một frame lạnh (28–34 ms một thread ở 60K node, ADR-0014) chỉ lúc rảnh.
4. **Vòng lặp render không I/O (sans-IO)**, để code thật chính là mô hình được kiểm. Stateright vét cạn tới độ sâu
   khoảng 12; chạy ngẫu nhiên kiểu shuttle sâu hơn hằng đêm.
5. **Sửa mã khoá width** bằng một từ nhãn tường minh; Kani giữ nó trong CI.
6. **Reactive:** mô hình Lean 4 của alien-signals (tiền lệ DBSP khoảng 5K dòng) cộng test khớp vét cạn quy mô nhỏ, khoảng
   5 phút CI, sâu hơn hằng đêm.
7. **CI:** mọi Scenario và bước stress chạy với cold check; test cây có seed chạy dưới proptest để thu nhỏ; mọi lỗi in
   ra bản ghi replay được.
8. **Luật metamorphic** cho layout: dời gốc, hoán vị các lần sửa độc lập. 47 fixture layout của Chrome vẫn là oracle cho
   ngữ nghĩa, tách khỏi tính tăng dần.
9. **ADR mới "Tính tăng dần được chứng minh theo cách dựng"**, dùng định nghĩa đúng và tối thiểu của à-la-carte cho mọi
   phase; đổi ADR-0012, 0013, 0014, 0015, 0016, 0023.

**Phạm vi của "đầu tiên"** (đã thu hẹp sau kiểm chứng của Codex). Phải nói rõ mỗi mục chứng minh cái gì, vì bốn loại bảo
đảm khác nhau: so với frame lạnh (test), model checking có giới hạn (Kani, Stateright tới độ sâu k), chứng minh trên mô
hình (Lean), và chứng minh trên code ship (chưa mục nào ở đây đạt). Khẳng định có thể là mới [I]: đường cập nhật
compositor suy ra bằng đại số và chứng minh tương đương với compose lạnh; cache layout có tập đọc đủ theo kiểu. "Toolkit
đầu tiên có tính tăng dần được chứng minh" nói chung là **chưa đứng được**.

### 6.7 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm:** harness Kani cho `cache.rs`: với mọi hai `LayoutInput` và mọi `Reads`, `Key::answers` kéo theo hai input
bằng nhau trên các trục đã đọc. Nó phải tái hiện va chạm khoá width trong vài phút. Sau đó sửa mã khoá và giữ harness
trong CI. Khoảng một ngày, và trả lời thẳng câu hỏi Kani có hợp với ta không.

**Rủi ro:**
- Đường suy ra hoặc memo có thể chậm hơn `replan` viết tay; đo bằng số lệnh (dao động dưới 1%, ADR-0023).
- Mô hình Lean có thể trôi khỏi code Rust; test khớp là cầu nối duy nhất.
- Chuyển thuật toán Taffy sang `In<'_>` là sửa lớn trên các file mà phiên khác đang làm.
- Kani không với tới closure và `dyn`, nên runtime reactive chỉ có mô hình cộng test, không có chứng minh trên code.
- Cold check ở release cần registry Fork ngoài debug, tốn bộ nhớ và allocation.
- Liveness kiểm tới độ sâu 12 chỉ phủ các lần chạy có giới hạn.

---

## 7. Dữ liệu cực lớn: list, bảng, chart

**Mục tiêu.** List và bảng tới hàng triệu dòng có chiều cao thay đổi theo nội dung, chart hàng triệu điểm, ở 120 Hz,
với scrollbar chính xác, neo cuộn, nhảy tới chỉ số, và accessibility trên các dòng chưa mount.

### 7.1 Ai gãy ở đâu

Mọi bên đều đoán dòng chưa đo bằng trung bình × số dòng (hoặc một gợi ý cố định), và gãy theo cùng ba kiểu: scrollbar
nhảy, nhảy tới chỉ số phải sửa nhiều lần, đổi chiều rộng thì vứt hết số đo.

- **Compose:** `animateScrollToItem` ước khoảng cách bằng `visibleItemsAverageSize() * indexesDiff`, đích trung gian kẹp
  2500 dp (ngưỡng huỷ riêng 1500 dp); từ vòng 2 nếu đích còn xa thì teleport tới cách đích 100 item
  (`LazyListState.kt:823`); bảng khoá phủ tối đa 230 item (khối 30 cộng 100 mỗi bên,
  `LazyLayoutNearestRangeState.kt:58`) [V, Codex sửa]. Android không có scrollbar lazy sẵn; scrollbar desktop không tới được cuối ở lần kéo đầu khi chiều cao thay
  đổi ([compose-mp #2679](https://github.com/JetBrains/compose-multiplatform/issues/2679)) [V].
- **Flutter:** `maxScrollExtent` đoán từ các con đang dựng, không cache; scrollbar nhảy
  ([#80445](https://github.com/flutter/flutter/issues/80445), [#97354](https://github.com/flutter/flutter/issues/97354));
  [#132216](https://github.com/flutter/flutter/issues/132216) báo giật nhưng chưa tách được nguyên nhân là ước lượng;
  [#129768](https://github.com/flutter/flutter/issues/129768): không tới cuối sau khi append, gọi lại là cách vòng; issue
  theo dõi [#52207](https://github.com/flutter/flutter/issues/52207) vẫn mở và ghi nhận lối thay `itemExtentBuilder` đã
  ship; PR cache #134864 đóng không merge [V, Codex].
- **Chrome:** `contain-intrinsic-size: auto` chỉ nhớ kích thước sau khi phần tử đã render một lần, nên lần hiện đầu vẫn thay
  placeholder bằng kích thước thật (con số "6.100 px" bản trước không tìm thấy trong nguồn [Codex]); CSSWG chốt bật
  `auto` ([#8407](https://github.com/w3c/csswg-drafts/issues/8407))
  ([csswg #5815](https://github.com/w3c/csswg-drafts/issues/5815), [#7807](https://github.com/w3c/csswg-drafts/issues/7807), TAG #876) [V]. Đổi lại DOM giữ mọi node nên find-in-page
  và a11y vẫn chạy, với bộ nhớ tỉ lệ n [I].
- **TanStack Virtual:** giật khi cuộn lên và một tuỳ chọn không có tác dụng ở các bản bị ảnh hưởng
  ([#1227](https://github.com/TanStack/virtual/issues/1227)); nhảy tới chỉ số trượt quá khi fling cảm ứng trên Android
  Chrome ([#1346](https://github.com/TanStack/virtual/issues/1346)) [V, Codex]. react-window có báo cáo lịch sử cùng lớp
  (#741; #883 đã sửa ở 2.3.2) [V].
- **Zed:** tổng chiều cao là `Pixels` (`f32`), nên quanh 2²⁴ px bước 2 px. Đo mọi dòng ở layout đầu (`measure_all()`) là
  **tuỳ chọn**; mặc định chỉ đo dòng nhìn thấy, và GPUI đã lưu tổng chiều cao khi bắt đầu kéo scrollbar
  (`references/zed/crates/gpui/src/elements/list.rs:183,747`) [V, Codex sửa]. Tức là Zed đã có "đóng băng extent khi kéo".
- **iced #3429:** người báo nói `List` đo lại mọi dòng khi đổi chiều rộng và GPUI cũng lag, nhưng maintainer nói upstream
  không có widget `List` như vậy; ticket không chứng minh cơ chế [Codex: không kiểm được]. (Bản trước dẫn Xilem #1373 cho
  Masonry; issue đó thật ra nói về `VirtualScroll` chiếm hết chỗ trong flex, đã bỏ [Codex].)
- **Pretext** (Cheng Lou, 3/2026) cài đo và layout dòng bằng JavaScript, dùng `measureText` của Canvas cho chiều rộng, để
  biết chiều cao trước khi mount; virtualization là một ứng dụng ([repo](https://github.com/chenglou/pretext)) [V, Codex].
  Đó là trần của web: không sở hữu engine chữ thì phải dựng lại một phần nó.
- **Chart:** (bản trước dẫn plotly.js #7955 như lỗi float32 ở timestamp lớn; Codex kiểm thì issue đó dùng x nhỏ và y
  khoảng 0,00001, maintainer nghi nhiễu số, **không** chứng minh chẩn đoán float32; đã bỏ dẫn chứng này. Vấn đề độ chính
  xác f32 với timestamp lớn vẫn là [I], và ADR-0018 đã giải bằng x `i64`.) plotly-resampler decimate ở server (MinMaxLTTB,
  khoảng 1.000 điểm mỗi view), mỗi lần pan hay zoom là một vòng gọi [V].
- **Accessibility:** không adapter nền tảng nào của AccessKit đọc `row_count`/`row_index`
  ([AccessKit #871](https://github.com/AccessKit/accesskit/issues/871), dự án này mở); UIA của Windows có
  `ItemContainerPattern` và `VirtualizedItemPattern` cho item chưa hiện thực hoá, AccessKit không map [V].

### 7.2 Vì sao chưa ai giải

- **Mỗi bên chỉ sở hữu một lớp.** Virtualizer web không đo được nếu không mount và không chạy được engine chữ ngoài
  thread. List lazy của Flutter và Compose dựng dòng bằng composition trên UI thread, nên đo dòng chưa thấy nghĩa là chạy
  code app. Chrome giữ mọi node để có a11y và tìm kiếm.
- **Ước lượng nằm đúng chỗ scrollbar đọc**: thumb và extent là cùng một số, nên ước lượng tốt hơn cũng làm thumb nhảy
  [I].
- **Chiều cao lưu theo vị trí, không theo dòng dữ liệu**: sort hay filter là mất, khoá dời xa thì chỉ tìm trong cửa sổ
  nhỏ (Compose tối đa 230 item, ta 1024) [V].
- **Nghiên cứu layout trên GPU không hợp UI.** Superconductor (Meyerovich, LASH-C 2013) chạy layout attribute-grammar trên
  GPU cho treemap 94.000 điểm bỏ phiếu (số lá); một pass prefix sum định cỡ buffer vẽ trước khi điền [V, Codex]. Bản trước
  nói "chạy được vì không có shaping": nguồn không chứng minh điều đó [Codex]. Chưa thấy ví dụ đầu-cuối nào shape OpenType
  tổng quát (script, fallback) trên GPU; Slug và HarfBuzz GPU chỉ mã hoá hoặc raster outline, bản Slug WebGPU shape trên
  CPU [V, Codex: không tìm thấy, không phải chứng minh không có].

### 7.3 Code của ta hôm nay

**Đã tốt** [V]: `list/extent_tree.rs` là B+ tree bền vững (fan-out 32) của `Run { rows: u64, height: u32, measured }`,
chiều cao theo 1/64 px, tổng `u64`, seek và set một dòng O(log n); splice xoá nhiều run khác nhau có thể tốn O(k log R)
hoặc dựng lại O(R) [V, Codex]. Vị trí cuộn là `Anchor { row, key, offset }`; offset tuyệt đối `u64` vẫn được tính để seek,
nhảy theo tỉ lệ và đặt cửa sổ. Yêu cầu neo được giữ tới khi các dòng quanh nó đã đo, nên nhảy tới chỉ số chính xác. Scene
stress 10M dòng (số đo lịch sử trong ADR-0014): p50 1,7 ms, layout 0,14 ms. Scene đó có hai dạng dòng (một trên bảy dòng
cao), vẫn tách thành nhiều run nên có thử cây, nhưng không thử phân bố chiều cao dài và bất thường, nơi chi phí và bộ nhớ
xấu nhất (`crates/shadcn/examples/stress/scenes.rs:509-540`) [V, Codex].

**Vấn đề:**

1. **O(số run) trên đường cuộn** [V, tôi đã kiểm]: sau mỗi layout có đo dòng mới, `virtual_list.rs:939` gọi
   `ext.set_estimate(units(mean))`, và `set_estimate` (`extent_tree.rs:289`) gom mọi run, dựng lại vector, dựng lại cây.
   Ước lượng lượng tử hoá không đổi thì trả về ngay; đổi thì đi qua mọi run, cấp phát vector tạm và dựng lại tuyến tính,
   kể cả khi không còn run chưa đo [V, Codex]. Lướt vào các dòng mới chiều cao khác nhau chạm đường này; tần suất ("mỗi
   vài frame") chưa đo [I]. "Không gì tỉ lệ n" của ADR-0014 không giữ ở đây.
   **Thêm (Codex):** trung bình chỉ đếm một vị trí ở lần đo đầu; chiều cao đổi sau đó cập nhật cây nhưng không cập nhật
   tổng mẫu, nên ước lượng có thể cũ ngay cả khi không sort (`virtual_list.rs:925-939`) [V].
2. **Bộ nhớ:** 10M dòng đã đo, chiều cao khác nhau, khoảng 160–200 MB [I].
3. **Thumb tự dời** khi trung bình đổi, dù nội dung đứng yên [I].
4. **Đổi chiều rộng reset hết** (`virtual_list.rs` `reset(estimate)`), cùng lỗi với iced [V].
5. **Chiều cao gắn theo vị trí:** `View` (`list/view.rs`) là hoán vị `u32`; không gì đổi khoá `ExtentTree` khi nó đổi.
   Sau một lần sort cùng độ dài, chiều cao đã đo của các vị trí ngoài màn hình gắn với dòng khác; vị trí đang mount được
   sửa lại khi layout. Bảng triệu dòng của stress dùng lại list qua sort, nên chạm được; cần chiều cao không đều mới thấy
   lỗi [V, Codex xác nhận].
6. **Khoá tìm trong cửa sổ** −512…+511 dòng (`KEY_SEARCH`); trượt thì rơi về vị trí cũ [V].
7. **Một ước lượng toàn cục.** ADR-0014 đề xuất ước lượng theo loại nội dung, và mục "as built" của nó ghi rõ bản làm ra
   chỉ có một trung bình [V, Codex]. Đây là việc còn lại có ghi nhận, không phải lời hứa bị phá.
8. **O(n) trên UI thread trong primitives:** typeahead quét tuyến tính `items.label(i)` (`primitives/src/typeahead.rs:168`);
   `Keymap::step` đi bộ qua item disabled [V/I].
9. **Không đo ngoài thread hay lúc rảnh**: dòng chỉ được đo khi dựng, và dựng chạy component (`!Send`) [V].
10. **Chart** (`chunk/series.rs`, ADR-0018) đi trước ngành: M4 trên GPU qua pyramid 64-nhánh, x `i64` trừ chính xác 64
    bit, hover trên CPU cùng frame; **line chart** 100M điểm GPU p50 1,17–1,44 ms ở 4K (chưa có số cho scatter dày). Hở:
    **scatter yêu cầu x không giảm**: `Series` ghi rõ hợp đồng này và `append`/`set` kẹp x (`[10, 5]` thành `[10, 10]`,
    `series.rs:688`, `chunk/plot.rs:31-35`) [V]. Đây là giới hạn có tài liệu, không phải lỗi (Codex sửa); nhưng scatter 2D
    thật sự cần một kiểu dữ liệu khác. Scatter cull và nén trước khi vẽ, nên mỗi điểm *còn lại* một instance. Chart chỉ có
    tóm tắt chữ bật theo lựa chọn, chưa có cây a11y có cấu trúc (5.2).

### 7.4 Ta có gì mà họ không có

Engine chữ của riêng ta, shaping cache tách khỏi ngắt dòng (ADR-0019), pool layout mỗi thread một `TextSystem`: đo chữ
ngoài thread không cần clone kiểu Pretext. Sum tree bền vững: snapshot O(1) gửi đi gửi về. `Collection` là handle
signal thuộc scope; worker nhận một `Snapshot` bền vững (`Send` với `T` phù hợp), mailbox mang kết quả về. Mô hình neo giữ
vị trí nội dung theo dòng, nhưng extent còn nuôi chọn cửa sổ, kẹp cuộn, nhảy theo tỉ lệ và kích thước list vừa nội dung,
không chỉ thumb [V, Codex sửa]. Runtime
chunk dữ liệu GPU có pyramid, prefix sum có thứ tự, readback và tham chiếu CPU khớp từng bit [V].

### 7.5 Thiết kế đề xuất

**A. Kho extent theo dòng dữ liệu, ước lượng O(1)** (đổi ADR-0014). Chiều cao mỗi dòng dữ liệu trong một `Vec<u32>` dạng
cột (0 = chưa đo), 4 B/dòng. Cây vị trí là B-tree trên khối 256 vị trí; lá là "k dòng chưa đo thuộc lớp c" (không tốn
chỗ) hoặc một lát `[u32]`. Summary giữ `rows`, `measured_rows`, `measured_extent`, `unmeasured[c]` cho K lớp cố định;
`extent = measured_extent + Σ unmeasured[c]·est[c]` tính khi seek. Đổi ước lượng thành O(K), bỏ vấn đề 1; bộ nhớ trần
khoảng 4,1 B/dòng; ước lượng theo lớp (loại nội dung) đúng lời hứa của ADR-0014.

**B. Sort và filter sinh chiều cao cùng view.** Worker dựng `View` đồng thời dựng cây vị trí từ chiều cao theo dòng dữ
liệu (O(n) ngoài thread, dựng hàng loạt tuyến tính) và hoán vị ngược; cả hai về cùng lúc qua mailbox. Neo tìm theo khoá
O(1) ở mọi khoảng cách, bỏ `KEY_SEARCH`.

**C. Đo dòng thuần, `Send`, lúc rảnh** (ADR mới, nối với layout có kiểu ở review §12.2). List có thể cho `measure:
fn(&T, width) -> RowShape`: một công thức chỉ có dữ liệu (padding, cột hoặc hàng chữ với `ResolvedText`, hộp cố định).
Pool layout tính trên `Snapshot<T>` với `TextSystem` riêng, theo thứ tự: mẫu phân tầng khắp list (mỗi dòng thứ k, để
trung bình từng lớp không lệch từ sớm), rồi lan ra từ viewport, rồi phần còn lại. Khi dòng mount, chiều cao thật so với
dự đoán; lệch thì đếm trong `FrameReport` và là lỗi ở debug, như cold check. Đổi chiều rộng thì pool chỉ ngắt dòng lại,
không shape lại (khoảng 20M từ, dưới 10 ns mỗi từ, 8 core: 20–40 ms) [I]. Có ngân sách điện năng (bài 8); dừng khi biên
sai số của extent dưới ngưỡng.

**D. Extent đổi không bao giờ dời thứ người dùng đang nhìn.** Nội dung đã neo nên không dời. Thumb có luật liên tục: sửa
extent mà người dùng không gây ra chỉ áp (a) khi thumb ẩn, hoặc (b) rải qua nhiều frame, mỗi frame thumb dời tối đa 1
device px. Kéo thumb thì đóng băng ánh xạ tới khi thả. Nhảy tới chỉ số vẫn chính xác, và nhờ B thì O(log n) qua cả sort
và filter.

**E. Truy cập mức dữ liệu trên dòng chưa mount** (đổi ADR-0021, ADR-0025). `query(collection, predicate, fields, range)`
chạy trên pool, trả về khoá; `reveal(key)` dùng hoán vị ngược rồi `scroll_to`. Typeahead và bước bỏ qua disabled chuyển
sang đây; typeahead có chỉ mục nhãn đã sort dựng cùng view. Trên Windows, adapter cài `ItemContainerPattern.FindItemByProperty`
và `VirtualizedItemPattern.Realize` từ `Items`; đây là việc upstream, cần người dùng đồng ý.

**F. Chart** (đổi ADR-0018). Scatter có cấu trúc 2D: điểm theo thứ tự Morton hoặc Hilbert, pyramid min/max y theo khối x
hoặc pyramid tile k-d; bỏ kẹp x. Khi số điểm mỗi pixel vượt ngưỡng, một compute pass đếm theo device pixel bằng atomic
số nguyên (tất định vì chỉ là đếm), prefix sum trên GPU cho histogram cân bằng làm thang màu (`eq_hist` của Datashader),
trong một frame, cùng scene. Pyramid đã trả min/max của mọi khoảng trong O(log n), nên `query(series, x-range,
granularity)` trả bảng có kiểu cho agent và cây kiểu Olli (chart → series → khối → điểm) cho trình đọc màn hình, ở mọi
mức zoom, không thêm gì trên GPU.

**Lên GPU:** decimate và vẽ đường (đã có); cull và đếm mật độ scatter; thước tổng quan (đánh dấu kết quả tìm hay
selection trên 10⁶ dòng, gom vào pixel của track); tuỳ chọn ô lưới số với `tnum` (chữ số bằng advance nên đặt vị trí
theo công thức, không cần shape) [I].

**Ở lại CPU, có lý do:** tổng chiều cao (hit test, neo, a11y, phím cần nó trong cùng frame; bản trên GPU trả lời muộn
1–2 frame, cùng lý do ADR-0018 giữ hover trên CPU, và cây CPU đã O(log n)); shaping chữ (rẽ nhánh, phụ thuộc fallback,
chưa ai làm được trên GPU); đo dòng component (cần layout engine). Chỉ xét lại nếu đo thấy chậm: ngắt dòng lại trên GPU
khi resize, chấp nhận thumb trễ 1–2 frame.

**Phạm vi của "đầu tiên"** (đã thu hẹp sau kiểm chứng của Codex). Từng mảnh đều có tiền lệ: cache đo theo khoá
([CellMeasurer](https://github.com/bvaughn/react-virtualized/blob/master/docs/CellMeasurer.md)), đo list ngoài main
thread ([Texture](https://texturegroup.org/docs/containers-astablenode.html)), đóng băng extent khi kéo thumb (GPUI),
hiện thực hoá item a11y ảo ([UIA](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-implementingvirtualizeditem)),
gom dữ liệu lớn ([Datashader](https://datashader.org/user_guide/Performance.html)), cây chart a11y (Olli). Điều có thể
là mới [I] là **hành vi tổng hợp đo được**: tất cả cùng lúc ở quy mô 10⁶–10⁷ dòng có chiều cao theo nội dung, nhảy tới
chỉ số chính xác qua sort và filter, thumb dời có giới hạn, và a11y/query trên dòng chưa mount. Thí nghiệm 7.6 là thứ
chứng minh, không phải danh sách tính năng.

### 7.6 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm:** trước hết là sự thật nền, vì hôm nay không gì đo được kiểu gãy này. Thêm scene stress `list_chat_1m`: 1M
dòng, chiều cao đuôi dài (1–40 dòng chữ wrap, có seed), cộng một oracle chỉ cho test, layout lạnh mọi dòng. Kịch bản:
fling liên tục qua 200K dòng; kéo thumb 0 → 0,5 → 1,0; `scroll_to_row` 100 chỉ số ngẫu nhiên; kéo đổi chiều rộng ±200
px; đổi sort. Đo mỗi frame trong `FrameReport`:

| Chỉ số | Mục tiêu |
|---|---|
| UI p95 ở 120 Hz | ≤ 4 ms |
| Nội dung nhảy (dịch chuyển y của dòng neo mà người dùng không gây ra) | 0 px |
| Thumb gián đoạn (dời mỗi frame không giải thích được bằng delta cuộn) | ≤ 1 device px |
| Sai số extent so với oracle | < 1% sau 2 s rảnh, chính xác trong thời gian hữu hạn |
| Sai số đáp khi nhảy tới chỉ số / số frame để ổn định | 0 px / ≤ 2 |
| Tỉ lệ dòng mount có chiều cao lưu sai sau sort | 0 |

Ngoài ra đo thời gian trong kho extent, số run hoặc khối, bộ nhớ heap. Chạy trên code hôm nay, rồi với A, A+B, A+B+C.
Dự đoán baseline [I, tới khi chạy]: thời gian `set_estimate` tăng theo số dòng đã lướt, thumb nhảy, tỉ lệ sai sau sort
> 0.

**Rủi ro:**
- Công thức `RowShape` và render thật có thể lệch. Giảm bằng so sánh lúc mount, lỗi ở debug, fallback về ước lượng cho
  list không có công thức; layout có kiểu (review §12.2) có thể sinh công thức từ chính component.
- Đo lúc rảnh đua với sửa dữ liệu: kết quả phải mang phiên bản snapshot và bị bỏ nếu cũ; tương tác với replay (mục 4).
- Điện năng: đo nền 10M dòng tốn CPU, cần chính sách ngân sách (bài 8).
- Hoán vị ngược và chỉ mục nhãn thêm 8+ B/dòng; 100M dòng khoảng 1,3 GB, cần chính sách phân trang.
- Map UIA là việc upstream; macOS không có tương đương cho ảo hoá [I].
- Atomic số nguyên chỉ độc lập thứ tự cho phép đếm, không cho tổng màu; tham chiếu CPU khớp từng bit vẫn bắt buộc.
- Luật thumb có thể thấy ì khi sửa lớn; cần thử với người dùng.

---

## 8. Chữ đẹp hơn trình duyệt

**Mục tiêu.** Chữ trong app shadcn-rs tốt hơn Chrome và toolkit native ở các mặt đo được: ngắt dòng và căn đều, độ đậm và
tương phản của glyph, fallback font tất định, caret trong văn bản hai chiều, và tốc độ sửa đoạn văn lớn.

### 8.1 Ai thiếu ở đâu

- **`text-wrap: pretty` của Chrome hẹp.** `ScoreLineBreaker` là bản port `OptimalLineBreaker` của minikin (Android), giới
  hạn `kMaxLinesForOptimal = 4`, `kMaxLinesForBalance = 6`, tắt dưới line-clamp
  (`references/chromium/.../score_line_break_context.h:17-18`) [V]. `pretty` của WebKit chạy trên cả đoạn (dòng cuối
  ngắn, độ lởm chởm của lề, gạch nối liên tiếp), nhưng chưa xử lý "sông" khoảng trắng, và tự nhận tốn kém với đoạn hàng
  nghìn dòng ([WebKit](https://webkit.org/blog/16547/better-typography-with-text-wrap-pretty/)) [V].
- **Chi phí của minikin là đoán:** phạt gạch nối `0.5·size·scaleX·width`, ×4 ở mức Normal, kèm chú thích "TODO: Replace
  with a better value after some testing" [V]. Một phép đo bên thứ ba thấy ngắt tối ưu gần rẻ như ngắt tham lam (3,23 so
  với 3,34 ms cho 79 layout, [OxyOS #194](https://github.com/OxyHQ/OxyOS/pull/194)) [V], nên lý do "tốn kém" để không
  ngắt tối ưu là yếu.
- **Gạch nối trên Chrome:** Windows và Linux chỉ có `hyphens: auto` từ M88, vẫn cần `lang` [V].
- **Gamma và tương phản:** Skia dùng hằng số gamma theo nền tảng lúc biên dịch; trên Windows bỏ qua ClearType tuner tới
  Chrome 132 (1/2025); mờ khi khử răng cưa xám vẫn mở ([crbug 40037624](https://issues.chromium.org/issues/40037624)) [V].
  Bảng mask gamma của Skia chọn hàng hiệu chỉnh **chỉ từ màu chữ**, giả định nền có độ sáng ngược lại [V]. Giả định đó
  gãy với chữ muted trên nền muted, xám vừa trên card có màu, và dark mode.
- **FreeType:** stem darkening của **autohinter** (experimental) tắt mặc định; engine Adobe mặc định thì bật cho CFF, Type
  1, CID [V, Codex sửa]. FreeType cảnh báo rằng không có blend tuyến tính cộng gamma thì
  glyph làm đậm trông "nặng và nhoè" (`references/freetype/include/freetype/ftdriver.h:423-455`) [V]. Tranh cãi trên Linux
  tồn tại vì không ai sở hữu cả blend lẫn làm đậm.
- **Optical size:** các trình duyệt dùng CSS px cho `opsz` theo quy ước cài đặt; CSS Fonts 4 cho phép xét cả tín hiệu khác
  như mật độ pixel ([csswg #4430](https://github.com/w3c/csswg-drafts/issues/4430)) [V, Codex]. Chuyện "Chrome từng nhân
  theo DPR trên Windows" không kiểm được từ issue đã dẫn. Blitz thấy Parley áp `trak` của SF ở cỡ đã nhân scale ([blitz #1360](https://github.com/DioxusLabs/blitz/pull/1360))
  [V].
- **Flutter:** gõ chậm dần sau vài nghìn dòng ([#114158](https://github.com/flutter/flutter/issues/114158),
  [#128575](https://github.com/flutter/flutter/issues/128575): 3.000 dòng mất khoảng 6 s trên CanvasKit); caret bidi sai
  chỗ ([#123424](https://github.com/flutter/flutter/issues/123424)) [V].
- **Fallback:** Parley #899 là tra cứu âm lặp lại và trượt cache khi xen kẽ khoá script và `Hani` lúc shape (#900 thêm
  cache âm; không liên quan cuộn như bản trước ghi); #928 fallback theo locale bỏ qua mặc định của script [V, Codex].

**Vì sao chưa ai giải:** trình duyệt bị ràng bởi tương thích web, vì đổi cách ngắt đoạn văn có sẵn là dời layout của cả
web. Nên Chrome chỉ chấm điểm ngắt dòng khi được bật, tối đa bốn dòng. CSS đã chốt opsz theo px. Skia đoán gamma từ màu
chữ vì màu không nằm trong khoá glyph và nền không biết lúc dựng glyph. Toolkit native chỉ sở hữu một lớp (Flutter gọi
SkParagraph qua FFI; macOS bỏ subpixel AA từ Mojave, không có tham số chỉnh) [I]. Không ai kiểm chữ so với một tham
chiếu: WebKit nói không đo sông, hằng số của minikin chưa kiểm [V].

### 8.2 Code của ta hôm nay

- **Shaping:** Parley 0.13 với `complex-scripts` (ICU4X tách từ theo từ điển cho CJK, Thái, Khmer, Lào, Myanmar), shape bằng
  harfrust 0.15 (`crates/gui/Cargo.toml`) [V]. Không dùng mô hình LSTM của ICU [V].
- **Ngắt dòng** là ngắt tham lam của Parley; `pretty` và `balance` là tìm nhị phân trên *chiều rộng*
  (`text/mod.rs:3896-3956`) [V]. Cách này không biểu diễn được chi phí gạch nối hay độ lởm chởm [I].
- **Lỗi: `pretty` không làm gì trên đoạn hơn 4 dòng** [V, tôi đã kiểm]. `mod.rs:3901` trả về ngay trừ khi cả đoạn có
  2..=4 dòng, trong khi ADR-0005 dòng 18 và chú thích `PRETTY_MAX_LINES` đều nói Chromium xử lý *bốn dòng cuối* của đoạn.
  Code khác cả ADR lẫn Chrome. **Codex xác nhận ADR đúng:** Blink bỏ dòng cache cũ nhất, thêm dòng mới, trượt cửa sổ 4
  dòng tới khi cuối đoạn lọt vào; test sáu dòng của Blink kiểm hai lần trượt
  (`references/chromium/.../score_line_breaker_test.cc:109`). Muốn khớp Blink còn phải theo điều kiện chấm điểm của nó
  (chỉ khi dòng cuối ngắn không ngắt được, hoặc hai dòng gạch nối liền trước đó) và phân biệt line-clamp kiểu cũ và mới
  [V, Codex]. `balance` dừng ở 6 dòng, khớp Chromium [V].
- **Chưa có:** căn đều (`TextAlign` không có `Justify`, `style.rs:701-711`, dù Parley có `Alignment::Justify`), gạch nối,
  chữ dọc, `lang` trên `ResolvedText` (`style.rs:1129-1150`), nên không có từ điển gạch nối, không chọn được dạng Hán theo
  locale (zh-Hans hay ja) [V].
- **Cache shape:** đoạn được chia tại `\n` thành piece (`mod.rs:612`, `756`); mỗi piece shape nguyên ở scale 1,0 (tránh lỗi
  `trak` của Blitz); khoá gồm nội dung và style, không có chiều rộng (`ShapeKey`, `mod.rs:250`); kết quả ngắt cache trong
  `Shaped::settled` [V]. Một lần sửa shape lại cả piece: textarea 10.000 dòng thì ổn, một đoạn 50 KB thì shape lại cả 50
  KB mỗi phím [V cấu trúc, I chi phí].
- **Optical size:** sáu file `Inter-opszN.ttf` đã được thay bằng một file biến `assets/Inter.ttf` có trục `opsz`, không có
  `wdth` (`fonts.rs:18`). `opsz` đặt bằng cỡ CSS px (`mod.rs:661-676`), giống `auto` của Chrome; mỗi tập giá trị trục được
  intern thành `u16` trong `GlyphKey.axes` [V].
- **Atlas glyph:** `GlyphKey` gồm font, ppem thiết bị, trục, 2 bit vị trí x theo phần tư pixel, cờ hinted/embolden/màu
  (`fonts.rs:33-53`); hinting bằng autohinter của skrifa dưới 2×, không bao giờ trên macOS; stem darkening khớp CoreText
  trên macOS (ADR-0019) [V].
- **Blend và gamma:** blend trên giá trị sRGB đã mã hoá, surface chỉ 8 bit unorm (`gpu/mod.rs:472-479`); coverage qua bảng
  `SkMaskGamma` 256×8, hàng chọn từ màu chữ (`gamma.rs:172-186`, `shaders.wgsl`, `slug.wgsl`) [V]. Tức là "đúng như
  trình duyệt", chưa đúng gamma, và tương phản được đoán từ màu chữ.
- **Fallback:** fontique với `system_fonts: true` (`fonts.rs:203-222`), phụ thuộc font đã cài, phiên bản OS, và việc
  resolve lười theo script; không ghi lại ở đâu [V] (đã là lỗ hổng replay ở 4.1).

### 8.3 Ta có gì mà họ không có

1. **Màu nền thật lúc ghi scene.** Theme là giá trị có kiểu (ADR-0022), scene biết fill đặc gần nhất dưới mỗi item chữ.
   Skia, theo thiết kế, không biết.
2. **Cả đường ống:** bộ ngắt, shaper, rasterizer, bảng gamma, blend. Ta đổi được mặc định cho component văn xuôi của mình
   mà không tốn chi phí tương thích web.
3. **Harness so với Chrome** (`verdict/text.sh`: OKLab, mực, nét), mở rộng được thành rasterizer tham chiếu.
4. **Cache theo piece, khoá không có chiều rộng**: ngắt tối ưu là hàm cache được của (piece, chiều rộng).
5. **Hệ style có kiểu**: áp được ghi đè giãn cách WCAG 1.4.12 cho toàn app và *chứng minh* không component nào bị cắt.

### 8.4 Thiết kế đề xuất

Mỗi mục được chấm là giá trị thật hay mánh, kèm lý do.

- **A. Bộ ngắt toàn đoạn của riêng ta (thật).** Quy hoạch động kiểu Knuth–Plass/minikin trên advance của cluster và cơ hội
  ngắt của Parley, cắt nhánh "bestHope". Ép từng dòng đã chọn bằng `set_line_max_advance` theo dòng trên `BreakLines`
  của Parley (`mod.rs:3870-3875`) [I]. Thay `pretty` và `balance` dạng tìm chiều rộng, sửa luôn lỗi trên 4 dòng.
  - **Mặc định cho component văn xuôi** (`Prose`, `Typography`, mô tả, văn bản dài). Nhãn UI ngắn giữ tham lam,
    balance hoặc pretty, vì ở đó verdict so với Chrome.
  - **Chi phí:** phương sai độ lởm chởm; dòng cuối ngắn hơn ⅓; gạch nối liên tiếp; và một hạng "sông": khoảng trắng giữa
    từ chồng nhau ở các dòng kề. WebKit để việc này là FIXME; nhưng **Holkner (2006) đã tối ưu ngắt đoạn theo diện tích
    khoảng trắng chồng nhau giữa các dòng kề** ([luận văn, §3.3.5](https://gwern.net/doc/design/typography/tex/2006-holkner.pdf#page=19))
    [V, Codex], nên hạng sông không mới; phần chưa rõ có tiền lệ là đưa nó vào ngắt dòng tương tác, mặc định.
  - **Giới hạn:** quy hoạch động chính xác tới khoảng 200 dòng mỗi piece; dài hơn thì quy hoạch theo cửa sổ, và khi đó
    phải nói rõ là xấp xỉ.
  - **Relayout tăng dần, sửa lại** [Codex bác bản trước]: tối ưu toàn cục có thể đổi cả các điểm ngắt *trước* chỗ sửa, và
    cùng một offset có thể mang state khác (khoảng trắng dòng trước, trạng thái gạch nối, thống kê cộng dồn mà hạng sông
    đọc). Vậy "chạy lại từ 2 dòng trước, dừng khi trùng offset" **không** tương đương giải lạnh. Phải định nghĩa đủ state
    của quy hoạch động và điều kiện dùng lại hậu tố; khi đang gõ thì đóng băng các dòng trên caret (đã nêu ở rủi ro) là
    chính sách có chủ ý, không phải tương đương.
  - **Ép điểm ngắt:** `set_line_max_advance` của Parley chỉ đặt giới hạn rộng, không bảo đảm đúng điểm ngắt đã chọn; cần
    đánh giá API ngắt theo số ký tự tại ranh giới atom của Parley (`line_break.rs:1270`) [V, Codex].
  - Đổi ADR-0005 (điều khoản pretty/balance), mở rộng ADR-0019; chỗ khác Chrome ghi vào `verdict/better.tsv` kèm số đo.
- **B. Gạch nối và `lang` (thật).** Thêm `lang` vào `ResolvedText` (ADR-0022); nó cũng điều khiển fallback theo locale và
  tách từ của ICU. Gạch nối bằng `hypher` (trie nhúng của Typst) làm cơ hội ngắt mềm cho A; chỉ bật mặc định khi căn đều
  hoặc cột rất hẹp (quy tắc của Typst).
- **C. Căn đều và vi chỉnh chữ.**
  - `TextAlign::Justify` (thật), map sang `Alignment::Justify`; bộ ngắt toàn đoạn là thứ làm căn đều chấp nhận được.
  - **Căn lề quang học** (thật): treo dấu câu và phần nhô của glyph tròn ra lề theo tỉ lệ từng glyph (bảng của
    microtype). Rẻ, thấy rõ, không cần font hỗ trợ.
  - **Giãn glyph qua `wdth`** (mánh nếu bật mặc định, loại): Inter không có trục `wdth`, và co giãn kiểu hz làm méo nét ở
    cỡ UI. Chỉ cho phép với font có trục `wdth`, tối đa ±2%.
- **D. Optical size theo kích thước vật lý (phần lớn là mánh).** Kích thước mm trong EDID thường bằng 0 hoặc sai trên TV,
  máy chiếu, dock [I]; font nay được tinh chỉnh theo quy ước px mà mọi trình duyệt dùng [V]; khoảng cách đọc, thứ thật sự
  quyết định optical size, không biết được. **Giữ:** `opsz` = CSS px như Chrome, cộng một tỉ lệ "optical scale" có kiểu
  cho theme kiosk hay TV. **Phần thật:** bảo đảm `opsz` không bao giờ theo DPR (lỗi Chrome từng có), có test trục không
  đổi khi scale đổi 1× → 2×.
- **E. Tương phản theo nền thật (thật, và là mục chính; không phải đầu tiên).** **Tiền lệ** [V, Codex]: SUI (10/2026)
  đã làm trên GPU, dùng độ sáng nền đặc thật cho grayscale và RGB nền thật cho LCD, atlas dùng chung, có fallback khi
  nền không chắc ([shader](https://github.com/sinomo-lab/sui/blob/d6cd95bd3ef28adbbad994122a2592af2cf2f467/crates/sui-render-wgpu/src/shaders/text_atlas_dual_source.wgsl#L68),
  [docs](https://github.com/sinomo-lab/sui/blob/d6cd95bd3ef28adbbad994122a2592af2cf2f467/docs/text-system.md#L392)).
  Đó là tiền lệ gần nhất; ta phải nói rõ ta làm tốt hơn ở đâu và đo thế nào. **Ràng buộc mà bản trước thiếu** [Codex]:
  - "Fill đặc gần nhất" chỉ là nền thật khi nó **phủ trọn dấu chân glyph** và vẫn nhìn thấy qua clip, các lệnh vẽ xen
    giữa và compositing. Không chứng minh được thì giữ cách đoán của Skia.
  - Một cặp độ sáng vô hướng **không** mã hoá được hiệu chỉnh phi tuyến độc lập của từng kênh RGB; muốn khớp blend RGB
    tuyến tính thì phải tính theo từng kênh (SUI làm vậy).
  - Stem darkening theo nền đổi **hình học** đã cache (`fonts.rs:669`), nên cần xử lý riêng trong khoá atlas hoặc khi
    raster; không thể chỉ đổi bảng.
  Thiết kế: item chữ mang nền đã chứng minh (RGB, không chỉ độ sáng) hoặc cờ "không biết"; bảng hoặc hàm hiệu chỉnh theo
  từng kênh; darkening tách khỏi atlas hoặc thêm vào khoá có chủ ý. Đổi ADR-0019 (điều khoản gamma) và ADR-0016.
- **F. Fallback tất định (thật).** Lúc khởi động, đóng băng một manifest fallback: (script, lang) → hash nội dung file font,
  theo thứ tự của fontique. Mọi fallback resolve qua nó, không resolve lười giữa lúc cuộn (Parley #899). Mỗi run ghi (dải
  codepoint, hash font) vào bản ghi replay (mục 4); replay trên máy khác dùng chế độ "chỉ font đi kèm" và báo lỗi rõ khi
  hash lệch. Manifest phải là của ta, không phải cache của fontique.
- **G. Caret và selection bidi có property test (thật).** Proptest trên chuỗi trộn LTR/RTL/số/emoji. Bất biến: đi trái/phải
  theo hình thăm mỗi điểm caret đúng một lần mỗi chiều; logic↔hình đi vòng khớp; hình chữ nhật selection phủ đúng các
  grapheme đã chọn, không chồng; Backspace xoá đúng một grapheme hoặc một dấu kết hợp.
- **H. Kiểm so với rasterizer tham chiếu (thật).** Coverage diện tích chính xác f64 trên outline của skrifa, chạy trong
  test; so glyph atlas (không hint) và glyph Slug với nó: sai số coverage lớn nhất và trung bình theo ppem và vị trí, có
  ngưỡng, trên lavapipe. "Khớp Chrome" thành "khớp coverage chính xác".
- **I. Accessibility của chữ.**
  - **Ghi đè WCAG 1.4.12** (thật): giá trị môi trường đặt line height 1,5×, khoảng đoạn 2×, letter spacing 0,12 em, word
    spacing 0,16 em. Mọi demo shadcn trong verdict chạy dưới nó, có kiểm không node chữ nào tràn hay bị cắt. Đó là bằng
    chứng tuân thủ mà chưa toolkit nào ship [I].
  - **Font cho người dyslexia** (mánh): các nghiên cứu có đối chứng không thấy lợi ích khi đọc [I, chưa kiểm lại]. Phần
    có bằng chứng là ghi đè giãn cách.
- **J. Sửa đoạn văn lớn (thật).** Piece lớn được shape theo piece con tại ranh giới an toàn để nối (safe-to-concat) của
  harfrust, mức bidi resolve một lần cho cả piece. Ngưỡng 4 KB chỉ là điểm kích hoạt; nó không bảo đảm có điểm nối an
  toàn hay giới hạn ngữ cảnh cần shape lại [Codex]. Cần kho run của riêng ta bên cạnh `Layout` của Parley; đây là mục
  "Revisit when" có sẵn trong ADR-0019.
- **K. Chữ dọc** (thật cho xuất bản CJK, không cho UI shadcn): cần `vert`/`vrt2`, `vmtx`/`VORG`, hộp dòng dọc, Parley không
  có. ADR riêng, ngoài thí nghiệm này.
- **L. Chữ HDR và gam màu rộng** (sau): phụ thuộc surface float (`gpu/mod.rs:474` chỉ nhận 8 bit) và một mốc trắng SDR, nếu
  không chữ sẽ loá. Ghi là phụ thuộc của bài 7.

**Phạm vi của "đầu tiên"** (Codex bác cả hai khẳng định chính của bản trước). Tương phản theo nền đã có ở SUI; hạng sông
trong ngắt đoạn đã có ở Holkner 2006. Những gì còn lại [I, chưa kiểm được là mới]: manifest fallback đóng băng ghi vào
replay; coverage chữ kiểm trong CI so với tham chiếu diện tích chính xác. Kiểm WCAG 1.4.12 trên các demo là **bằng chứng
cho các trạng thái đã thử**, không phải chứng minh mọi app tuân thủ. Giá trị của mục này là làm đúng và đo được trên cả
chồng của ta, không phải là người đầu tiên.

**Sửa thêm** [Codex]: manifest fallback `(script, lang) → hash file` chưa đủ cho replay; cần byte font chính xác, chỉ số
face trong collection, và danh sách ứng viên có thứ tự hoặc face đã resolve cho mỗi run. Property test bidi cần định
nghĩa affinity của caret ở vị trí mơ hồ, chính sách selection cho ligature và run hình, và chính sách xoá của Backspace.
Giãn `wdth` không "méo nét" một cách tất yếu (Thành, TUGboat); giữ tắt mặc định vì Inter không có trục đó.

### 8.5 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm: tương phản theo nền (E)**, thay đổi nhỏ nhất mà kết quả thấy rõ nhất: một bảng, một field trên item, harness
đã có. Mở rộng `text-sample.tsv` với các cặp thật của theme (`foreground`/`background`, `muted-foreground`/`muted`,
`primary-foreground`/`primary`, sáng và tối, 11–24 px, 1× và 2×). Vẽ ba biến thể: (a) hàng Skia hôm nay; (b) blend tuyến tính
(view sRGB offscreen) làm tham chiếu cảm nhận; (c) bảng 8×8×256 mới với độ sáng nền theo item.

| Chỉ số | Mục tiêu |
|---|---|
| Độ đậm nhất quán: tỉ lệ độ tối cảm nhận của nét giữa theme sáng và tối cho cùng glyph | \|1 − tỉ lệ\| ≤ 3% (đo (a) trước; dự đoán 10–20% [I]) |
| Đọc được ở tương phản vừa: APCA Lc của nét đã vẽ so với Lc của outline lý tưởng, trên `muted-foreground`/`muted` | ≥ 90% lý tưởng |
| Không thoái lui nơi Skia đoán đúng: tỉ lệ mực và OKLab so với Chrome | trong ngưỡng ADR-0019 (0,95–1,05; OKLab ≤ 0,10) |
| Chi phí | texture 16 KB, thêm 1 u8 mỗi item chữ, thời gian frame `docs --bench` không đổi ngoài nhiễu |

**Song song: bộ ngắt (A).** Sửa lỗi `pretty` trên 4 dòng trước (lỗi tương đương với Chrome), rồi prototype ngắt toàn đoạn
trên các trang văn xuôi của docs. Đo độ lởm chởm (độ lệch chuẩn độ đầy dòng) so với Chrome tham lam và Chrome `pretty`,
số dòng cuối ngắn, điểm sông, p95 thời gian ngắt mỗi piece (< 50 µs cho 20 dòng), số dòng ngắt lại mỗi phím trong đoạn
2.000 dòng.

**Rủi ro:**
- E: nền không biết dưới ảnh, blur, lớp trong suốt, gradient, nên độ đậm có thể nhảy ở ranh giới biết/không biết. Khác
  Chrome có thể làm verdict fail, mỗi ca cần một dòng `better.tsv` có số đo. Nền chuyển động làm đổi hàng mỗi frame: rẻ,
  nhưng không được nháy.
- A: ép ngắt bằng max-advance theo dòng có thể lệch quy tắc khoảng trắng cuối dòng và treo của Parley. Quy hoạch động
  phải dùng đúng cùng chiều rộng, không thì một dòng tràn một epsilon (Chrome có `AddEpsilon` vì vậy). Ngắt tối ưu dời
  các dòng phía trên khi người dùng gõ: đóng băng các dòng trên caret khi đang sửa, tối ưu lại lúc rảnh.
- B: trie gạch nối tăng kích thước binary theo ngôn ngữ; nối `lang` là thay đổi xuyên suốt ADR-0022.
- F: thứ tự nội bộ của fontique có thể không ổn định giữa các phiên bản.
- J: shape theo piece con có thể để lộ đường nối (kerning, ligature qua ranh giới); cần fuzzer so với shape nguyên đoạn.
- Chung: Parley đổi API mỗi minor (ADR-0019); mọi mục cần móc vào việc ngắt (A, C) làm tăng chi phí nâng cấp.

---

## 9. Màu đúng từ đầu tới cuối, gam màu rộng và HDR

**Mục tiêu.** Màu theme viết bằng OKLCH hiện đúng trên màn sRGB, Display P3 và HDR; gradient nội suy đúng không gian; ảnh
có profile được chuyển đúng; ảnh và video HDR nằm cạnh UI SDR mà UI không xám đi hay loá; tất định và kiểm được.

### 9.1 Số đo của chính ta

Agent chạy mọi token OKLCH trong `crates/shadcn/src/theme/data.rs` qua phép đổi OKLab → sRGB [V]: 837 mục, 193 khác nhau,
**70 token ngoài sRGB, không token nào ngoài Display P3**. Token vượt xa nhất `oklch(0.606 0.25 292.7)` lố 0,064 ở một
kênh tuyến tính. Bảng màu của upstream là bảng P3, hôm nay ta kẹp nó về sRGB từng kênh.

| Đo trên 70 token, màn sRGB | Giá trị |
|---|---|
| ΔE_OK trung bình do kẹp | 0,0089 |
| ΔE_OK lớn nhất | 0,026 (vàng `oklch(0.852 0.199 91.9)`) |
| Số token ΔE_OK > 0,02 (khoảng một bước vừa thấy) | 3 |
| Lệch hue lớn nhất | 5,0° |

**Kết luận:** trên màn sRGB, gamut mapping so với kẹp chỉ khác chút ít. Phần mất lớn là trên màn P3, nơi ta vứt màu mà
màn hiện được. Chromium có đường hiển thị gam rộng (`skia_renderer.cc:1485`), nên trên Mac gam rộng ta có thể **dưới**
chuẩn tham chiếu; nhưng fixture Chrome của ta chỉ thử canvas sRGB, nên đầu ra thật của 70 token này trên Chrome P3 chưa
được kiểm [V, Codex]. Codex tính lại và xác nhận mọi con số trên (837 mảng, 193 bộ khác nhau gồm alpha, 191 bộ màu; ΔE_OK
trung bình 0,00887, lớn nhất 0,02645).

### 9.2 Ai gãy ở đâu

- **Gamut mapping của CSS:** [csswg #9449](https://github.com/w3c/csswg-drafts/issues/9449) "Channel clipping breaks
  author expectations, especially when using 'perceptually uniform' spaces"; spec chuyển sang giảm chroma giữ L và hue
  OKLCH, Chromium phản đối vì hiệu năng và tương thích, trình duyệt vẫn kẹp trong thực tế; tìm nhị phân của CSS bị chê
  chậm và không nằm đúng biên gam so với phương pháp giải tích của Ottosson [V].
- **HDR trong CSS:** `dynamic-range-limit` mặc định `no-limit` vì Chrome không đồng ý mặc định bị giới hạn; Apple muốn
  giới hạn mặc định (CSSWG HDR breakout 3/2025); media query theo lựa chọn người dùng vẫn mở (#14616) [V]. Web mặc định
  là loá.
- **Windows:** ở chế độ HDR, app SDR đi qua thanh trượt "SDR content brightness"; người dùng gặp desktop xám, bạc màu,
  và các hướng dẫn trái ngược nhau về chiều kéo [V]. Trong scRGB, 1,0 nghĩa là 80 nit; app không nhân theo mức trắng SDR
  sẽ quá tối, hoặc quá sáng nếu đoán [I].
- **Flutter** [V, Codex sửa bản trước]: gam rộng đến iOS trước (engine #39111 đã merge; #39580 đề xuất revert nhưng đóng
  không merge); `Color` gam rộng của framework đã ship (#127855), vẽ widget được trên backend hỗ trợ; Flutter 3.47 bật gam
  rộng cho macOS trên phần cứng đủ điều kiện (#186277); Android Impeller Vulkan vẫn đang làm (#127852). Lỗi thật trên
  đường đi: hạ gam khi window nil (#135033), crash định dạng Metal trên iPad (#133015), ma trận P3 áp lên giá trị đã mã
  hoá (#181717, sửa 2/2026). Impeller chọn texture `R32G32B32A32Float` cho gradient khi có stop gam rộng, RGBA8 khi không
  (`gradient_generator.cc:58-69`): một lựa chọn định dạng lưu trữ [V].
- **Android:** `Window.setDesiredHdrHeadroom` tồn tại vì headroom đầy "ảnh hưởng độ sáng cảm nhận của phần lớn nội dung
  SDR" (`Window.java:1352-1375`); SurfaceFlinger theo dõi `sdrWhitePointNits` và một tầng làm tối mỗi output
  (`OutputCompositionState.h:140-147`) [V]. Làm tối SDR dưới HDR là việc của compositor, app không làm được.
- **Linux:** `color-management-v1` vẫn ở `staging/`; Mutter thêm ở GNOME 48; KWin có mô hình đầy đủ hơn
  (`ColorDescription` với `referenceLuminance`, `maxHdrLuminance`, rendering intent, `kwin/src/core/colorspace.h`) [V].
  Kiểu monitor của winit không mang dữ liệu màu hay HDR (`winit-core/src/monitor.rs:151`) [V].
- **wgpu 30** có `SurfaceColorSpace` (Srgb, ExtendedSrgbLinear, DisplayP3, ExtendedDisplayP3, Bt2100Pq, Bt2100Hlg,
  ExtendedSrgb, PassThrough) và `Surface::display_hdr_info`; nhưng nit chỉ có trên Windows, headroom chỉ trên Apple,
  Vulkan ngoài Windows không có gì; không ghi metadata HDR; không tone hay gamut map ("việc của app"); vừa sửa Mac gam
  rộng hiện sRGB quá bão hoà (#10286) và crash khi cấu hình PQ/HLG/ExtendedDisplayP3 (#10175) [V].
- **Trình duyệt blend trong sRGB đã mã hoá** vì hai thập kỷ nội dung được chỉnh theo nó (alpha, khử răng cưa, bóng) [I].
  ADR-0011 của ta chép điều này "as browsers blend" (`docs/adr/0011…md:53`) [V]: đó là lựa chọn tương thích, không phải
  lựa chọn chất lượng.

**Vì sao chưa ai giải:** trình duyệt không đổi được blend hay giới hạn HDR mặc định mà không vỡ web. Flutter sở hữu
pipeline nhưng màu đi qua kiểu `Color` sRGB nhiều năm (#127855), nên gam rộng đến theo từng nền tảng. Compositor
(SurfaceFlinger, KWin, DWM) xử lý SDR trong HDR nhưng không biết pixel trong app nghĩa là gì. Hệ điều hành đưa ba mô hình
khác nhau (nit trên Windows, headroom trên Apple, độ sáng tham chiếu trên Wayland) và winit không đưa cái nào; chưa thấy
toolkit nào hợp nhất chúng [I]. Ít ai có harness tham chiếu chính xác từng pixel, nên gam rộng vỡ mà không ai thấy
(Flutter #181717).

### 9.3 Code của ta hôm nay [V]

- **Kiểu màu:** `Oklch` giữ chroma ngoài sRGB, hue thiếu, màu tương đối và `color-mix` của CSS Color 4
  (`crates/gui/src/color.rs`); đó là sự thật lúc viết. `Color` là sRGB đã mã hoá f32, alpha thẳng. **Đổi sang `Color` kẹp
  từng kênh về 0..1** "as Chrome draws it on an sRGB surface" (`color.rs:435-447`, tôi đã kiểm).
- **Theme** giữ `Palette<Oklch>` (`crates/shadcn/src/theme/palette.rs`), chỉ đổi sang `Color` ở `painted()`.
- **Record GPU đã mang màu dạng bốn f16** (`shaders.wgsl`, `wcolor`): giữ được giá trị ngoài 0..1, nhưng CPU đã kẹp
  trước đó.
- **Gradient** nội suy từng pixel trong OKLab với alpha premultiplied; `to_gamma` trong shader kẹp 0..1
  (`common.wgsl:195-201`); không chọn được không gian nội suy, **không có dither** (không có "dither" nào trong
  `crates/gui/src`, tôi đã kiểm). Màu động cũng trộn trong OKLab (`motion.rs`).
- **Surface:** chỉ `Bgra8Unorm` hoặc `Rgba8Unorm`, chú thích "Blending happens in the sRGB-encoded values, as browsers
  blend", `color_space: Auto` (`gpu/mod.rs:472-498`, tôi đã kiểm). Trên Metal, wgpu 30 gắn nhãn layer đó là sRGB (#10286),
  nên macOS chuyển đúng nhưng không bao giờ hiện P3 [I]. Texture offscreen là `Rgba8Unorm`.
- **Canvas bền vững** được `blit.wgsl` chép lên surface bằng một tam giác phủ màn và `textureLoad` thuần (tôi đã kiểm).
  **Đây là chỗ tự nhiên cho một phép biến đổi đầu ra theo màn hình, không tốn thêm pass nào.**
- **Gamma chữ** dùng bảng mask gamma của Skia theo màu chữ, hàm truyền sRGB trên macOS và Windows, gamma mũ 1,2 (lượng
  tử về 1,1875) trên Linux (`text/gamma.rs:81`) [V, Codex]; phụ thuộc blend đã mã hoá. Canvas và layer dùng `self.format`,
  nên trong cửa sổ có thể là BGRA; còn có mask `R8Unorm` [Codex].
- **Ảnh:** `Pixels` là RGBA8 premultiplied; decode PNG đặt `EXPAND | STRIP_16`, bỏ qua iCCP, cICP, gAMA, cHRM
  (`image.rs:232-262`); PNG là decoder duy nhất; ảnh lấy mẫu qua filter tuyến tính trên giá trị đã mã hoá.
- **Theo màn hình:** không truy vấn không gian màu hay HDR; `current_monitor` chỉ dùng cho tần số quét.
- **Test:** `tests/chrome_oklch.rs` kiểm sRGB chưa kẹp và byte đã vẽ so với Chrome. Verdict so chênh kênh lớn nhất với dung
  sai **48/255** (`examples/docs/verdict.rs:52-76`), không phải OKLab: quá lỏng để thấy lỗi màu dưới khoảng 19% mỗi kênh.
  Chỉ `text_sample.rs` báo OKLab trung bình và p95. `Color::contrast` cài WCAG 2.2.
- **ADR:** ADR-0022 cam kết "OKLCH colour … as CSS Color 4 computes them"; chưa ADR nào nói về gam, màn hình hay HDR.

### 9.4 Ta có gì mà họ không có

1. Màu tác giả viết còn sống tới lúc vẽ: token `Oklch`, record f16 mang được giá trị mở rộng.
2. Một uber-shader cộng một blit của ta: cả phép biến đổi đầu ra nằm trong một fragment shader, điều trình duyệt hay
   compositor không làm được bên trong app.
3. Không có nội dung web: mọi alpha, bóng, opacity trong component là của ta, chỉnh lại được.
4. Harness tham chiếu: verdict, `chrome_oklch`, đọc lại pixel, lavapipe trong CI.
5. API không gian màu và `DisplayHdrInfo` của wgpu 30; ta sở hữu phần phía trên (độ sáng tham chiếu của Wayland, sự kiện
   khi cửa sổ sang màn khác).

### 9.5 Thiết kế đề xuất

- **A. Hai miền màu, có kiểu ở ranh giới (thật).** Màu tác giả viết vẫn là `Oklch`. Màu làm việc là
  `Linear<Rec709Ext>`: ánh sáng tuyến tính, dải mở rộng, 1,0 = trắng SDR. Tag không gian bằng phantom type chỉ ở nơi byte
  qua ranh giới (ảnh đã decode, texture GPU, đầu ra surface, đọc lại). Tag trên mọi màu theme là mánh: một không gian làm
  việc duy nhất đã khiến trộn hai không gian là bất khả theo cấu trúc [I]. `Color` (sRGB mã hoá, đã kẹp) chỉ còn là kiểu
  đầu ra và kiểu test.
- **B và C, sửa lại sau kiểm chứng.** Bản trước đề xuất "canvas `Rgba16Float` giữ giá trị sRGB mở rộng đã mã hoá, blend y
  như hôm nay trong ≤ 1 LSB, rồi gamut map trong blit". **Codex bác cả hai vế** [V, Codex]:
  - *Không khớp từng LSB:* tám lần vẽ trắng alpha 1/1024 lên đen, UNORM8 làm tròn mỗi lần về 0 nên kết quả là byte 0;
    FP16 giữ phần tăng nên kết quả là byte 2. Lệch ngay cả với số học blend lý tưởng, chưa kể độ chính xác blend của phần
    cứng. (Bước của binary16 ngay trên 1,0 là 2⁻¹⁰, không phải 2⁻¹¹ như bản trước ghi.)
  - *Gamut map sau khi blend mã hoá mở rộng là sai về nghĩa:* xanh P3 50% alpha trên nền đen, blend trong Rec.709 mở rộng
    đã mã hoá rồi giải mã, cho P3 tuyến tính `(-0,004; 0,213; -0,004)`; blend trong ánh sáng tuyến tính cho `(0; 0,5; 0)`.
    Bộ map cuối làm được kết quả hiển thị được, nhưng không dựng lại được kết quả đúng, cũng không tái tạo được việc kẹp
    theo từng nguồn như sRGB.

  **Thiết kế sửa:** *định nghĩa compositing trước, rồi mới chọn định dạng canvas.*
  - Hai đường có ngữ nghĩa tường minh: (1) **đường SDR tương đương** giữ canvas UNORM8 mã hoá như hôm nay, nơi cần khớp
    byte với Chrome (verdict sRGB); (2) **đường gam rộng/HDR** dùng màu **tuyến tính premultiplied** trong canvas 16F cho
    mọi nội dung, nơi các hiệu ứng tác giả viết trong không gian mã hoá (`bg-black/50`, bóng, ring/50) được cho ngữ nghĩa
    tường minh, ví dụ blend mã hoá mô phỏng trong shader cho đúng loại overlay đó, thay vì để định dạng canvas quyết định
    ngầm.
  - Trần HDR theo nguồn (E) áp **trước** khi làm phẳng, khi còn biết nội dung nào là nội dung nào; blit cuối chỉ làm phép
    biến đổi đầu ra (primaries, neo trắng, OETF, dither), không gamut map nội dung đã trộn.
  - Gamut map áp **theo từng nguồn màu** (token, stop gradient, ảnh) khi nó vào không gian làm việc của màn đích. CSS
    Color 4 cho phép ba thuật toán SDR giữ L và hue OKLCH: Binary Search với Local MINDE, EdgeSeeker, Ray Trace; phương
    pháp của Ottosson dùng xấp xỉ cusp cộng tinh chỉnh Halley (con số "50 ALU" bản trước không có nguồn, đã bỏ) [V, Codex].
  - Lấy mẫu ảnh qua view `*UnormSrgb` **không** tuyến tính hoá đúng RGB đã premultiply trong không gian mã hoá [Codex];
    ảnh phải được lưu premultiplied tuyến tính nếu muốn filter tuyến tính.
- **D. Phép biến đổi đầu ra theo màn hình, và hợp đồng nền tảng.** wgpu **30.0.2** (bản ta khoá) có `Auto` cộng bảy không
  gian đặt tên tới `ExtendedDisplayP3`; **không có `PassThrough`** (chỉ có trong checkout tham chiếu mới hơn) [V, Codex].
  Headroom của Apple chỉ trên macOS và phải truy vấn trên main thread, tức platform thread của ta. Windows: scRGB 16F,
  **chỉ nội dung SDR** nhân theo nit trắng SDR ÷ 80. Wayland: phải thống nhất định dạng surface Vulkan với mô tả ảnh mà
  compositor ưa thích; PQ là một lựa chọn, không phải mặc định. Mã hoá surface, gam của màn và trắng giấy SDR là ba đầu
  vào khác nhau. Chọn lại khi cửa sổ sang màn khác, khi `ScaleFactorChanged`, hoặc khi `display_hdr_info` đổi.
- **E. Neo trắng SDR và chính sách HDR.** UI không bao giờ vượt trắng SDR của nền tảng. Nội dung HDR (ảnh, video, chart)
  có trần riêng mỗi node: `HdrLimit::{Standard, Constrained, Full}`, mặc định `Constrained` cho media tình cờ, với đường
  cong ghim trắng SDR, trần xác định, và tôn trọng cài đặt người dùng. WebKit đã cài giá trị khởi đầu có giới hạn, có
  ngoại lệ cho video [V, Codex]. 203 nit của BT.2408 là tham chiếu sản xuất HDR, **không** được đè trắng SDR của desktop
  [Codex sửa]. Ngân sách theo diện tích là ý của ta; API desired headroom của Android không quy định điều đó [V, Codex].
  Đây là mốc trắng SDR mà 8.4 L cần cho chữ HDR.
- **F. Ảnh theo mô tả màu của nó (thật).** Đọc iCCP, cICP, gAMA, cHRM, và EXIF trong JPEG, AVIF, HEIF; giữ dữ liệu 16 bit
  thay vì `STRIP_16`; chuyển một lần lúc upload vào không gian làm việc qua một CMS thuần Rust, bằng f16 khi nguồn gam rộng
  hoặc HDR, còn lại giữ `Rgba8UnormSrgb`.
- **G. Gradient (thật).** `Interpolation::{Oklab, Oklch(HueInterpolation), LinearRgb, Srgb}` có kiểu; bỏ kẹp ở
  `to_gamma` để stop giữa các màu P3 vẫn là P3; dither trong blit bằng một tile blue-noise cố định ±0,5 LSB theo độ sâu bit
  đầu ra, tất định vì tile là hằng.
- **H. Kiểm chứng (thật).** Phép biến đổi là một hàm thuần trong Rust (f64) và WGSL; khẳng định đọc lại GPU lệch ≤ 1 LSB
  so với mô hình Rust trên lavapipe và Metal; kiểm chuyển đổi ICC so với lcms2 hoặc skcms trong test (ngưỡng ΔE_OK ≤ 0,002
  [I]); thêm chỉ số OKLab trung bình và p95 vào verdict cạnh dung sai kênh 48.
- **I. Tương phản từ cặp đã hiện (thật; phần "a11y trong HDR" là mánh hôm nay).** Test chạy trên theme × scheme × mọi cặp
  chữ/nền `ContentStyle`, **sau** gamut map cho từng loại màn; báo cả WCAG 2 (chuẩn pháp lý) lẫn APCA Lc (vị thế APCA chưa
  chắc [I]). Không chỉ số nào mô hình HDR, nên không khẳng định được a11y "trong HDR".
- **J. Cửa sổ trải hai màn dùng một phép biến đổi (loại là mánh).** Một swapchain có một không gian màu, compositor của
  OS chuyển cho màn thứ hai [I]; vẽ tách đôi sinh đường nối mà không có lợi đo được. Phần thật là chọn lại phép biến đổi
  khi cửa sổ dời màn (D).

**ADR đổi:** 0011 (canvas 16F mã hoá mở rộng, blit là phép biến đổi đầu ra, phép ánh sáng tuyến tính được chọn và đo
từng cái), 0003 (không gian nội suy có kiểu, dither), 0022 (theme vẫn OKLCH, map lúc present, không ở `painted()`), 0019 và
8.4 (gamma hay coverage tuyến tính, mốc SDR cho chữ HDR), 0016 (định dạng canvas và chi phí), 0023 (kiểm màu), và ADR mới
"màu viết bằng OKLCH và trình bày theo từng màn hình".

**Phạm vi của "đầu tiên"** (Codex bác các khẳng định rộng). Tiền lệ [V, Codex]: GTK giữ OKLCH của CSS làm trạng thái
màu của renderer và theo trạng thái màu compositor Wayland ưa thích
(`references/gtk/gtk/gtkcsscolor.c:1233`); Chromium/Skia đã có chuyển đổi raster và compositing theo màn hình, và giới
hạn HDR ở mức paint (`cc/paint/paint_op.cc:1315`); blend tuyến tính và mã hoá là lựa chọn đã quen; Apple và Microsoft có
hướng dẫn neo trắng; Unity và Unreal có paper white cho UI; Frostbite (GDC 2017) đã làm FP16, compose UI, tone map và mã
hoá màn trong một pipeline. Không tìm thấy bằng chứng ưu tiên cho tổ hợp cụ thể: một toolkit cài trên cả ba backend
(Metal EDR, Windows scRGB, Wayland color-management) cộng các kiểm tra CI đề xuất. Giá trị của mục này là **làm đúng và
kiểm được**, không phải là người đầu tiên.

### 9.6 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm (sửa sau kiểm chứng): đường gam rộng tuyến tính, chỉ sRGB và P3, chưa HDR.** Giữ nguyên đường UNORM8 hôm
nay làm đường SDR tương đương. Thêm đường thứ hai: canvas `Rgba16Float` tuyến tính premultiplied; mỗi nguồn màu được gamut
map khi vào không gian của màn đích; overlay do tác giả viết dùng ngữ nghĩa blend mã hoá tường minh trong shader; blit
chỉ làm primaries, OETF, dither. Chọn ExtendedSrgbLinear hoặc DisplayP3 trên Mac P3. Ba loại test tách riêng: tương đương
SDR, khớp CPU/GPU của phép biến đổi, và trình bày P3 thật (chụp có nhãn P3 hoặc đo màn hình), với các lệnh vẽ bán trong
suốt chồng nhau để thử lượng tử hoá.

| Chỉ số | Mục tiêu |
|---|---|
| Đường SDR tương đương: mọi demo verdict so với hôm nay | khớp từng byte (cùng đường UNORM8) |
| Đường tuyến tính trên sRGB: chênh lệch so với đường SDR ở overlay tác giả viết | đo và báo, không giả định ≤ 1 LSB |
| P3: đọc lại canvas, áp mô hình Rust; ΔE_OK của 70 token so với giá trị viết | ≤ 0,002 (hôm nay: trung bình 0,0089, lớn nhất 0,026) |
| GPU so với mô hình Rust | ≤ 1 LSB, lavapipe và Metal |
| Chi phí GPU, vẽ lại toàn bộ ở 5K và 4K | ≤ +10%; frame chỉ vẽ damage +0 |
| Bộ nhớ canvas | gấp 2 (khoảng 59 → 118 MB ở 5K) [I] |
| Điện năng (`powermetrics`), trace 60 s nghỉ và cuộn | ≤ +5% |
| Banding: gradient xám L 0,85 → 0,95 trên 1.000 px, 8 bit | đầu ra có dither không có dải phẳng dài hơn khoảng 8 px |

**Rủi ro:**
- Băng thông và GPU cũ: canvas 16F gấp đôi bộ nhớ và lượt đọc của blit; một số GLES và iGPU cũ thiếu render target hoặc
  blend 16F. Fallback canvas 8 bit phải giữ cùng phép biến đổi (kẹp) để test còn ý nghĩa.
- f16: NaN hoặc Inf từ phép toán ngoài gam phải được chặn (`cbrt` có dấu trong `common.wgsl:203` đã xử lý số âm [V,
  Codex]).
- Nhãn sRGB của Metal: với `Auto`, Metal tự quản màu sau lưng ta (#10286), phép biến đổi của ta và của OS có thể áp hai
  lần; wgpu 30.0.2 không có `PassThrough`, nên phải chọn tường minh một không gian có nhãn.
- Dither ±0,5 LSB cố định một mình không bảo đảm giới hạn độ dài dải phẳng [Codex]; chỉ số banding là mục tiêu, không phải
  hệ quả.
- Ma trận màu chạy trong blit vẫn xử lý toàn bộ frame được trình bày, kể cả khi chỉ vẽ damage.
- Tín hiệu màn hình không tin được: nit trên Windows lấy từ EDID lạc quan, Apple chỉ cho headroom, Linux Vulkan không có
  gì tới khi ta tự nối Wayland; winit không có sự kiện đổi màu.
- Verdict so với Chrome trên P3: tham chiếu là ảnh chụp sRGB; P3 cần tham chiếu riêng (Chrome headless với profile P3 ép
  buộc [I], hoặc mô hình Rust).
- Chỉnh lại: chuyển coverage sang tuyến tính đổi độ đậm chữ và khử răng cưa, phải qua chỉ số của 8.4 E trước.
- Decoder: hỗ trợ ICC và cICP cho PNG, JPEG, AVIF thêm dependency và bề mặt cần fuzz.

---

## 10. Điện năng là chỉ số hạng nhất

**Mục tiêu.** App shadcn-rs tốn ít điện hơn trình duyệt, Electron và toolkit native cho cùng UI; điện năng được đo, có
ngân sách và có test chống thoái lui như thời gian frame.

### 10.1 Ai gãy ở đâu

- **VS Code** ([#22900](https://github.com/microsoft/vscode/issues/22900), 2017): 13% CPU khi focus và nghỉ trên MacBook
  Pro 13"; caret đổi mỗi 500 ms nhưng profile cho thấy Chromium làm việc ở **60 Hz**; về khoảng 0 khi mất focus [V,
  Codex sửa].
- **Flutter** ([#59327](https://github.com/flutter/flutter/issues/59327), mở từ 6/2020): caret TextField nhấp nháy tốn 6–10%
  CPU đều đặn trên macOS, 0% khi animation dừng; nhóm Flutter: "the cursor is a timer and for each frame we need to
  composite and paint the layer tree to the surface" (The Register 2021) [V].
- **Firefox 70 trên macOS** ([Mozilla gfx](https://mozillagfx.wordpress.com/2019/10/22/dramatically-reduced-power-usage-in-firefox-70-on-macos-with-core-animation/),
  Intel Power Gadget, MacBook Pro 2013, trừ nền 3,6 W): Google Docs nghỉ có caret 7,4 → 1,6 W; hình vuông quay 12,8 →
  2,9 W; cuộn 16,4 → 9,4 W. Nguyên nhân: Firefox 69 "redrew the entire OpenGL window on every change"; vẽ lại một phần "on
  its own is responsible for most of the power savings" [V].
- **Zed/GPUI:** [#66221](https://github.com/zed-industries/zed/issues/66221) (2026) cửa sổ thu nhỏ trên Windows vẫn vẽ lại
  mỗi vsync (2,08% một core khi hiện, 3,44% khi thu nhỏ; egui 1,98% → 0,42%) vì `RedrawWindow` gửi tới mọi cửa sổ sau
  mỗi vblank, bỏ qua `IsIconic`; [#23479](https://github.com/zed-industries/zed/issues/23479) khoảng 5% CPU nền trên macOS;
  [#29073](https://github.com/zed-industries/zed/issues/29073) di chuột làm GPU tốn 2–3 lần điện [V].
- **winit:** [#4668](https://github.com/rust-windowing/winit/issues/4668) (mở) trên Wayland vòng lặp quay `epoll_pwait`
  timeout 0 khoảng 9.400 lần/s (winit 0.30.13, wgpu 29); [#4038](https://github.com/rust-windowing/winit/issues/4038) (mở)
  khoảng 30% CPU cho ví dụ rỗng trên Windows [V].
- **Wayland, cửa sổ ẩn:** không có giao thức fifo-v1 thì FIFO của Mesa chờ frame callback mà surface ẩn không bao giờ
  nhận, có thể chặn mãi; Mesa 24.3 thêm fifo-v1 [V, nguồn thứ cấp].
- **Chrome:** throttling mạnh kiểm timer chuỗi đủ điều kiện một lần mỗi phút sau hơn 5 phút ẩn; Google báo "tới 5 lần ít
  CPU hơn" trong test nội bộ; con số trung vị 1% là của **thí nghiệm sau**, bắt đầu throttling sớm hơn trên trang nền nặng
  [V, Codex sửa: hai thí nghiệm khác nhau].
- **Số của hệ điều hành** (lời nhà cung cấp, cho việc nền, không phải UI): Apple gán 72% giảm hoạt động CPU cho coalescing
  **cộng App Nap và tối ưu khác** của Mavericks; 23% là giảm điện app nhờ App Nap; Microsoft báo EcoQoS giảm tới 90% điện
  CPU ban đầu [V, Codex].

**Các bên đã đổi tần số frame:**
- **Android 15 ARR:** view bỏ phiếu category tần số theo heuristic: cập nhật "nhỏ" nếu ≤ 7% màn hình hoặc vừa ô 40 dp
  vuông / 10 dp hẹp thì được LOW (`View.java:5813-5824`, `25785-25801`); "ngắt quãng" nếu hai khoảng frame gần nhất cộng
  ≥ 100 ms (`ViewRootImpl.java:423-424`); kiểm nghỉ ở 750 ms [V].
- **Chromium viz:** `FrameIntervalDecider` có matcher cho chỉ video, chỉ ảnh động, scrollbar mờ dần, tăng tốc khi input,
  cuộn chậm (`frame_interval_matchers.h:30-41`) [V]. **Và Chromium đã suy khoảng tick từ đường cong keyframe dạng bậc**,
  nhân theo tốc độ phát, hài hoà các khoảng đồng thời và gửi nhịp đó tới viz để throttle refresh; đường cong liên tục thì
  không throttle trên đường này (`cc/animation/keyframe_effect.cc:806`, `layer_tree_host_impl.cc:3352`) [V, Codex]. Android
  cũng map vận tốc nội dung thành phiếu tần số (`View.java:34308`). Apple phân xử tần số animation và nhận gợi ý theo từng
  animation; nghiên cứu render cũng đã có tần số thích ứng theo chuyển động (Dénes và cộng sự, SIGGRAPH 2020) [V, Codex].
- **GTK** dừng caret nhấp nháy sau `gtk-cursor-blink-timeout`, mặc định 10 s (`gtksettings.c:367-377`) [V].

### 10.2 Vì sao chưa ai giải

Bản trước viết "Electron, Flutter, GPUI vẽ cả surface mỗi frame": **sai** [Codex]. Flutter có đường repaint một phần với
damage cộng dồn theo buffer (`flow/diff_context.cc:104`); Chromium và Firefox dùng được layer native; báo cáo GPUI thu nhỏ
chỉ chứng minh chi phí invalidation. Các báo cáo trên là workload cụ thể, không phải định luật. Bản trước cũng viết "không
bên nào biết đường cong": sai, xem Chromium ở trên. Cái thật sự còn thiếu là một **giới hạn sai số không gian màn hình**
cho đường cong liên tục (không chỉ dạng bậc) [I]. Đo khó: Energy Impact của Apple là trọng số theo model (Nethercote 2015), tài liệu của Mozilla khuyên tránh
nó, một bài 2026 trên Apple Silicon thấy nó theo điện CPU mà không theo điện GPU; bot điện của Chromium dùng powermetrics
và ghi rằng năng lượng "very sensitive to noise" [V]. winit báo `Occluded` là "Unsupported" trên Android, Wayland,
Windows, Orbital (`references/winit/winit-core/src/event.rs:518`, nhánh master) [V].

### 10.3 Code của ta hôm nay

**Nghỉ: các thread của ta không tự thức** [V từng thread; cả process là I tới khi đo]. Platform thread `ControlFlow::Wait`
(`window.rs:701`); UI thread `rx.recv()` khi không có timer, không thì `recv_timeout(next_timer)` (`window.rs:412-429`);
render thread `rx.recv()` khi `!busy() || !visible || !called_back` (`compositor/thread.rs:325-334`); pool layout là rayon;
worker raster chờ condvar. Chưa đo: thread của winit, adapter AccessKit, driver wgpu/Metal.

**Phát hiện quan trọng nhất: khi có animation, mỗi frame blit và present toàn cửa sổ, kể cả khi không đổi pixel nào**
[V, tôi đã kiểm; **Codex xác nhận**, thu hẹp phạm vi: "mỗi vòng `Surface` đủ điều kiện giữ texture đã acquire" (cần
`busy()`, hiện, callback sẵn sàng; acquire có thể thất bại; headless dùng `Ticker` không có surface); blit là tam giác phủ
màn không scissor, lấy mẫu canvas mỗi pixel đích; 3,93 GB/s là lưu lượng blit logic, chưa đo băng thông thật hay điện]. Với `Pace::Surface`, `wait_vsync` gọi `gpu.acquire()` mỗi frame để lấy nhịp vsync, lấy texture của
swapchain; khi đó `holds_frame()` thành true (`gpu/mod.rs:1114`), nên `present = window && (damage || overlaid ||
holds_frame())` luôn true và một pass blit toàn canvas được thêm (`gpu/frame.rs:1555`). Chú thích "A frame with nothing
changed submits nothing" (`gpu/mod.rs:20-25`) chỉ đúng khi render thread ngủ. Khi bất cứ gì đang chạy, một spinner 16 px
hay pha giữ của animation, mỗi vblank tốn một lượt đọc và ghi toàn canvas cộng một present toàn cửa sổ. Ở 2560×1600 là
16,4 MB mỗi present; ở 120 Hz, đọc cộng ghi khoảng 3,9 GB/s [I, tính toán]. ADR-0016 kỳ vọng caret chỉ tốn "vài trăm pixel
cộng một blit". `present_with_damage` (wgpu #10152) chỉ cho Vulkan và EGL, Metal và DX12 bỏ qua; **wgpu 30.0.2 ta khoá
chưa có nó**, renderer của ta không dùng EGL; gợi ý damage vẫn cần ảnh hợp lệ đầy đủ, và damage rect của ta phủ việc canvas,
không theo dõi pixel nào hợp lệ trong từng ảnh swapchain [V, Codex].

**Caret:** nhấp nháy bằng timer UI thread, `BLINK = 0.53` s mỗi nửa chu kỳ, theo lưới pha chung (`paint.rs:41, 2053`);
**không bao giờ dừng** khi còn focus (không có timeout kiểu GTK) [V]. Mỗi lần đổi chạy một frame UI ghi lại chunk overlay
của caret, rồi qua lại giữa các thread khoảng bốn lần [I]. `input_otp` dùng `Animation::caret_blink()` (keyframe 1,25 s
lặp vô hạn, `motion.rs:454-471`), nên `animating()` luôn true và render thread vẽ mỗi vblank mãi mãi, dù 60% mỗi chu kỳ
là pha giữ [V đường code, I "mỗi vblank"].

**Animation chạy đúng tần số màn hình:** `Animation::spin()` và `pulse()` lặp vô hạn (`motion.rs:419-441`); motion không
composite gọi `clock::request_frame()` mỗi frame; interval lấy từ `refresh_rate_millihertz`; không chọn tần số, không dùng
`CAFrameRateRange` hay tương tự [V].

**Present mode và GPU:** `PresentMode::AutoVsync`, `desired_maximum_frame_latency: 1` (`gpu/mod.rs:487-490`);
`PowerPreference::default()` là `None` (`gpu/mod.rs:465`), và với `None` wgpu-core bỏ bước sắp theo ưu tiên, lấy adapter đủ
điều kiện đầu tiên; thứ tự backend và lọc vẫn áp; **HAL Metal tự sắp danh sách theo `(low_power, headless)`**, nên "adapter
OS liệt kê đầu tiên thắng" là sai [V, Codex sửa]. GPU rời vẫn có thể được chọn; `LowPower` ưu tiên GPU tích hợp đủ điều
kiện.

**Bị che:** `Occluded` đi tới cả hai thread và render thread ngủ khi ẩn [V]; Wayland bị chặn theo frame callback [V]. Nhưng
Windows và Wayland không nhận `Occluded` từ winit (winit 0.30.13 ta khoá cũng vậy) [V]. Khi acquire thất bại, thread ngủ
rồi vẫn compose, submit tuỳ có việc (`thread.rs:486-491, 619-621`). Nhưng **thu nhỏ không bảo đảm acquire thất bại**: đường
DX12 của wgpu 30.0.2 chờ frame latency rồi lấy back buffer, không kiểm `IsIconic` [V, Codex]. Chi phí của GPUI #66221 đến
từ việc gửi thông điệp, nên không dự đoán được chi phí thu nhỏ của renderer ta. winit đã có `Window::is_minimized()` an toàn
(dựa trên `IsIconic`) cho thu nhỏ; bị che hoàn toàn là trạng thái khác.

**Ưu tiên thread:** render thread trên macOS chạy ưu tiên cố định 47 (`SCHED_RR`), chọn theo đo tick lỡ dưới tải 30–93;
chi phí điện năng của ưu tiên thời gian thực cho frame giá trị thấp như caret chưa bao giờ được đo [V].

**Proxy đã có:** ADR-0023 I5 đếm lệnh mỗi frame (`perf_event`, `proc_pid_rusage`); `Drawn` mang `submitted`, `uploaded`,
`damage`, `damage_rects`; `chunk/timing.rs` cho thời gian GPU [V].

### 10.4 Ta có gì mà họ không có

- **Biết đường cong của mọi animation:** `Animation` có keyframe, thời lượng, easing, và compositor lấy mẫu trên render
  thread. Ta biết trước khi nào pixel không đổi (pha giữ, kết thúc đã ổn định, nhấp nháy dạng bậc), và chặn trên được
  một điểm dời bao xa mỗi giây (tốc độ quay × bán kính, tốc độ dịch).
- **Biết damage chính xác ở cả hai đầu:** đã có damage rect của canvas, nên blit và present có thể giới hạn theo nó.
- **Sở hữu cả ba thread:** đếm được lần thức ở từng điểm chờ của ta, tất định, trong CI.

### 10.5 Thiết kế đề xuất

| Ý | Đánh giá | Lý do |
|---|---|---|
| **A. Chỉ present khi pixel đổi.** Tính *trước khi acquire* xem cửa sổ có cần cập nhật không (kể cả gỡ overlay và surface bị invalidate); không thì bỏ acquire, blit, present. Nhịp vsync tách khỏi acquire: `NSView.displayLink` (macOS 14+), compositor clock của Windows, frame callback trên Wayland (giữ cổng callback cho commit thật, và cho frame bị bỏ một đường thức ở thay đổi kế). Present có thay đổi vẫn tính blit đầy đủ cho tới khi biết pixel hợp lệ theo từng buffer hoặc dùng layer giữ riêng [Codex] | Thật, lớn nhất | Bỏ chi phí toàn cửa sổ ở pha giữ và frame compositor không làm gì. Số liệu Firefox ủng hộ hướng này, nhưng độ lớn là của đường render cũ của Firefox. Nối nguồn vsync native cần một ranh giới wrapper an toàn để giữ `forbid(unsafe_code)` trong `gui` |
| **B. Damage tới được OS.** `present_with_damage` trên Vulkan/EGL; trên Metal đặt nội dung nhỏ tần số cao (caret, spinner) vào sublayer `CAMetalLayer` riêng, hoặc vài layer theo ô như Firefox | Thật | Không làm thì đúng là lỗi của Firefox 69 |
| **C. Phiếu tần số suy từ đường cong.** Với mỗi node đang chuyển động, tần số cần là ước số nhỏ nhất của tần số màn giữ chuyển động trong ε device px mỗi frame, hoặc 0 khi đang giữ; lấy max trên mọi node; nhấp nháy dạng bậc cần 2 frame mỗi chu kỳ. Bỏ vblank trên render thread; xin OS một khoảng khi có API | Thật; mới chỉ ở phần giới hạn sai số cho đường cong liên tục | Chromium đã làm cho đường cong dạng bậc (10.1). Codex: cần **deadline của bước kế hoặc lúc hết giữ**, giới hạn riêng cho opacity, và giới hạn chuyển động hợp cho transform lồng nhau; chỉ lấy max các phiếu có thể bỏ sót bước lệch pha hoặc đánh giá thấp chuyển động gộp. Tiết kiệm chưa chứng minh: trên màn tần số cố định ta tiết kiệm lượt vẽ, không tiết kiệm refresh của panel |
| **D. Chính sách caret.** Dừng nhấp nháy sau 10 s nghỉ (GTK), dừng khi cửa sổ mất focus, vẽ nhấp nháy bằng hai frame mỗi lần đổi trên render thread, không cần frame UI | Thật, rẻ | Sửa cả lớp lỗi của VS Code và Flutter |
| **E. Bảo đảm nghỉ trong CI.** Sau N giây nghỉ: 0 frame compose, 0 submit, 0 present, ≤ k lần thức mỗi thread ở điểm chờ của ta | Thật | Tất định, không cần phần cứng; bắt được thoái lui kiểu Zed #66221 |
| **F. Ngân sách điện năng theo loại frame** (nghỉ, caret, spinner, cuộn, toàn bộ): present/s, byte blit và present, ms GPU bận mỗi s, lần thức/s, lệnh/s. Joule chỉ ở lab: powermetrics trên Mac, RAPL qua powercap trên Linux, E3/SRUM trên Windows | Thật cho proxy; joule trong CI là mánh | CI thuê không có bộ đếm điện ổn định, nhiễu lấn át hiệu ứng 5% |
| **G. GPU tích hợp mặc định** (`LowPower`, cho đè) | Thật trên Mac Intel hai GPU và laptop Windows; không tác dụng trên Apple Silicon | wgpu với `None` không sắp theo ưu tiên (HAL Metal tự sắp theo low_power) |
| **H. Tự xử lý bị che.** `Window::is_minimized()` của winit (an toàn, dựa trên `IsIconic`) cho thu nhỏ; xử lý bị che hoàn toàn riêng; không compose khi ẩn; bỏ vòng `sleep` khi ẩn | Thật | Bảo đảm cửa sổ ẩn không vẽ gì; không cần `unsafe` |
| **I. Ưu tiên theo loại frame.** Frame caret và spinner chạy QoS thấp hơn frame input và cuộn | Đáng thử | Ưu tiên cao cố định có thể giữ việc trên P-core [I]; chưa đo |
| **J. Giảm chất lượng theo pin** | Phần lớn là mánh khi đã có A–D | Che lãng phí thay vì bỏ nó; tối đa là tôn trọng Low Power Mode như `prefers-reduced-motion` |
| **K. Gộp timer giữa component** | Nhỏ | Caret đã chung một lưới pha, OS đã gộp; chỉ cần thêm leeway cho timer |

**ADR đổi:** 0015 (nguồn vblank không qua acquire, phiếu tần số, ưu tiên theo loại frame, bị che), 0016 ("cửa sổ nghỉ
không submit gì" thành "frame không có damage không present gì", damage tới OS, sublayer), 0023 (bất biến nghỉ, ngân sách
proxy điện năng mỗi scene stress, cột năng lượng lab trong `bench.json`), 0007 (tần số cần là thuộc tính của đường
cong), 0011 (`PowerPreference`).

**Phạm vi của "có thể mới"** (đã qua kiểm chứng Codex, thu hẹp thêm). Tiền lệ: Chromium suy nhịp từ đường cong dạng bậc;
Apple phân xử tần số và nhận gợi ý theo animation; Android/Compose nhận phiếu và map vận tốc; Flutter tách nhịp display
link khỏi việc vẽ; game và nghiên cứu render dùng ước số tần số và tần số thích ứng theo chuyển động; Firefox đưa damage
tới Core Animation; GTK có timeout nhấp nháy; bot điện của Chromium đo lần thức khi nghỉ. Điều có thể còn mới [Codex:
không kiểm được] chỉ là **chọn deadline từ đường cong liên tục được giữ lại, dưới một giới hạn sai số không gian màn hình
tường minh**, cộng bất biến nghỉ/caret trong CI.

### 10.6 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm: "sổ cái nghỉ và caret"**, scene stress trong cửa sổ thật, chưa đổi code. Mỗi ca 60 s: (1) trang tĩnh, (2)
ô nhập có focus, (3) một `spinner`, (4) `input_otp` có ô trống; mỗi ca ở trạng thái hiện, bị che, thu nhỏ; trên MacBook
ProMotion và laptop Windows.

| Chỉ số | Mục tiêu sau A+C+D (baseline là thứ ta dự đoán sẽ thấy) |
|---|---|
| Compose / submit / present mỗi s, ca 1 | 0 / 0 / 0 |
| Present mỗi s, ca 2 | ≤ 2 mỗi 1,06 s khi nhấp nháy; 0 sau 10 s; hôm nay ≈ 1,9 |
| Present mỗi s, ca 3 | ≤ phiếu đã tính (dự đoán ≤ 60 trên 120 Hz); hôm nay = tần số màn [I] |
| Present trong pha giữ, ca 4 | 0 trong 60% mỗi chu kỳ |
| Byte blit mỗi present | ≈ diện tích damage × 4 B, không phải cả canvas |
| Lần thức/s mỗi thread (bộ đếm của ta; context switch tự nguyện trên Linux) | ca 1: 0 từ thread của ta |
| Thu nhỏ/bị che, ca 3–4 | 0 compose |
| mW tổng của powermetrics (process trừ nền) so với cùng trang trong Chrome, Electron, `NSTextField` native | chỉ báo cáo; mục tiêu ≤ Chrome ở ca 2–4 |

**Rủi ro:**
- Lấy nhịp vblank từ display link thay vì acquire chặn có thể đưa lại jitter mà ADR-0015 đã đo; nhịp ProMotion/VRR vẫn
  chưa biết.
- Bỏ vblank có thể thành giật trên panel tần số cố định nếu đặt ε sai.
- Sublayer Metal kéo theo lỗi thứ tự compose, màu, resize; một phép so sánh DirectComposition của Firefox trên GTX 1050
  tốn hơn khoảng 56% (13,3 W so với 8,5 W), chênh ít hơn trên Iris 550 [V, Codex sửa].
- Lỗ của winit 0.30: không có `Occluded` trên Windows hay Wayland; lỗi quay #4668 sẽ hiện thành lần thức của ta.
- Nhiễu powermetrics và khác biệt giữa model Mac.
- Hạ ưu tiên render thread có thể đưa lại các tick muộn ADR-0015 đã đo.
- `LowPower` có thể chọn iGPU yếu và lỡ frame ở scene 5K lớn.

---

## 11. Nhiều màn hình, mỗi màn hình một nhịp

**Mục tiêu.** App có nhiều cửa sổ trên nhiều màn hình, mỗi cửa sổ theo đúng nhịp màn hình của nó (60/120/144 Hz,
ProMotion/VRR, scale, không gian màu, headroom HDR); kể cả khi cửa sổ dời màn hay trải hai màn, khi cắm rút màn, gập
máy; và việc của cửa sổ này không làm lỡ frame của cửa sổ kia.

**Thực tế hôm nay:** shadcn-rs chỉ có **một cửa sổ**. Với phần lớn app một cửa sổ, giá trị nằm ở ca **di chuyển** (laptop
cộng màn rời, gập máy, cắm rút); nhiều cửa sổ là thứ hai.

### 11.1 Ai gãy ở đâu

- **Chrome trên Windows:** thread vsync DXGI chờ trên output của màn **chính** (`MonitorFromWindow(nullptr,
  MONITOR_DEFAULTTOPRIMARY)`, `ui/gl/vsync_thread_win_dxgi.cc`), với chú thích "DwmGetCompositionTimingInfo is no longer
  guaranteed to align with the primary monitor" [V; Codex: chỉ đúng cho đường DXGI, bản hiện tại chọn đồng hồ
  DirectComposition khi bật]; crbug 40409726 báo rAF trên màn phụ chạy theo Hz màn chính [Codex: không kiểm được, tracker
  không hiện nội dung]. "Electron thừa hưởng" không có bằng chứng (electron#26460 nói chuyện khác) [Codex].
- **Chrome và Safari trên macOS:** viz có `SetVSyncDisplayID` ("this can change, e.g, as a window moves across displays"),
  `UpdateVSyncDisplay` nối lại `DisplayLinkMac` (`references/chromium/components/viz/common/frame_sinks/begin_frame_source.h:240-249`)
  [V]. WebKit [173434](https://bugs.webkit.org/show_bug.cgi?id=173434) mở từ 2017 (ban đầu về iPad 120 Hz); comment sau
  báo màn rời 144 Hz cạnh laptop bị kẹp, gập máy thì hết. Nhưng comment 60 báo Safari 17.5 cho 120 Hz trong và 165 Hz
  ngoài khi mở nắp, nên đây là **báo cáo lịch sử**, không chứng minh Safari hiện còn giới hạn [V, Codex sửa].
- **Flutter:** [#146249](https://github.com/flutter/flutter/issues/146249) (mở, P3): "we render all views on each frame.
  This incorrectly assumes that all views are on the same display"; mỗi engine có một `Animator` (`engine.h:1111`); PR GTK
  [#196776](https://github.com/flutter/flutter/pull/196776) (**chưa merge**): "Multi-view/multi-monitor currently uses the
  fastest display among the views because the engine has one animator"; diff có test tần số danh nghĩa 60–240 Hz, đổi
  tần số và pha từ presentation feedback cho một view [V, Codex].
- **Zed/GPUI:** PR [#60696](https://github.com/zed-industries/zed/pull/60696) (merge 7/2026) thay CVDisplayLink mỗi cửa sổ
  bằng "one immortal CVDisplayLink per display" mà cửa sổ đăng ký vào, vì crash use-after-free và rò một link mỗi lần
  resize/kích hoạt/che [V]. Vẫn gãy: [#67004](https://github.com/zed-industries/zed/issues/67004) (mở 11/2026) trên Mac ba
  màn 120/60/160 Hz cửa sổ đen sau **resize** (bản trước viết "sau khi dời màn": sai), dời màn không cứu được, CPU 0%,
  window server báo cửa sổ không ở trên màn; nguyên nhân là occlusion gate chỉ là giả thuyết [V, Codex sửa];
  [#63217](https://github.com/zed-industries/zed/issues/63217) (đóng, not planned) occlusion chặn khởi động khi khoá màn,
  bỏ gate thì lộ lỗi **tạo** link −6661; [#67295](https://github.com/zed-industries/zed/issues/67295) app GPUI độc lập,
  đổi view sau 300 s nghỉ, đường `CAMetalDisplayLink` có 9/24 lần quá 500 ms, tệ nhất 58.676 ms, so với tệ nhất 74 ms khi
  ép `CVDisplayLink`; không thử dời màn [V, Codex].
- **Windows DWM:** trước Windows 10 2004, Hz hỗn hợp kéo màn nhanh xuống; sau đó DWM được cho là compose ở Hz cao nhất,
  còn giật khoảng 6,9 ms với 144 trên 60 [Codex: không kiểm được mốc phiên bản và con số; Chromium có ghi một thay đổi
  timing DWM theo màn nhanh nhất ở các bản Windows 11 sau]. `DwmGetCompositionTimingInfo`: "Starting with Windows 8.1,
  this parameter [hwnd] must be set to NULL", nên API này không chọn được cửa sổ hay màn;
  `DCompositionGetTargetStatistics` cho thống kê composition theo target (Windows 11 22000+); `IDXGIOutput::WaitForVBlank`
  chờ trên output được chọn [V]. Bản trước ngụ ý Windows cũ không có thời gian theo swapchain: **sai**, DXGI có frame
  statistics cho swapchain flip-model hoặc fullscreen, với giới hạn độ tin cậy khi nhiều màn đã ghi trong tài liệu [Codex].
- **Wayland:** presentation-time chỉ đồng bộ được "only one output at a time", khuyên output chứa phần lớn nhất của surface
  **hoặc giữ output trước đó** (`presentation-time.xml:141-143, 237-243`) [V, Codex]; KWin chọn output tần số cao nhất mà
  surface chạm (`kwin/src/scene/surfaceitem_wayland.cpp:246-256`) và gọi `frameRendered` trước guard view chính, với TODO
  "also only send frame callbacks on the primary view" (`:271-275`) [V]. Bản trước viết "nhận callback theo hợp các tần số":
  **sai** [Codex]. Callback là một lần; lần vẽ đủ điều kiện đầu tiên rút hết callback đang chờ (`kwin/src/wayland/surface.cpp:511`),
  lần sau cần client đăng ký lại, nên không có callback ở mọi tick của hợp.
- **X11:** bản trước viết "compositor đồng bộ mọi đầu ra về một tần số, thường là thấp nhất": **sai như quy tắc chung**
  [Codex]. X Present cho phép nhắm CRTC và lịch MSC; một compositor có thể chọn một đồng hồ repaint và có báo cáo giật
  khi Hz hỗn hợp, nhưng không có chính sách "thấp nhất" chung.
- **wgpu:** trong wgpu-core **30.0.2** ta khoá, `Queue::present` giữ `device.command_indices.write()` suốt lượt present
  của hal (`present.rs:352-356`; số dòng trước là của checkout tham chiếu) [V, Codex], nên trên device dùng chung, một
  present bị chặn chặn submit và present của mọi cửa sổ khác trên device đó [V, Codex]; acquire của hal không giữ khoá đó
  (`:168-180`) [V]. [wgpu#1451](https://github.com/gfx-rs/wgpu/issues/1451): present chặn trên NVIDIA là quan sát driver
  cũ; [wgpu#9019](https://github.com/gfx-rs/wgpu/issues/9019) giải thích vì sao acquire dưới FIFO không phải tín hiệu vblank
  tin được [V, Codex]. bgfx#1867 (đóng): bộ đếm FPS nhảy 60→120 trên Mojave khi mở cửa sổ thứ hai; "vsync chỉ chạy mỗi
  lần gọi thứ hai" là giải thích của người báo [V, Codex].
- **VR mirror:** FreeFalcon PR #52 (đóng, không merge; desktop 60 Hz, headset 120 Hz): `CPU_FRAME` 16,56–16,60 ms khi bật
  vsync mirror, 8,30 ms khi tắt [V, Codex]; Unity để SDK VR điều khiển và bỏ qua `vSyncCount` [V].

**Vì sao chưa ai giải:** một đồng hồ. Animator duy nhất của Flutter và pacesetter của SurfaceFlinger giả định một đồng
hồ; trong SurfaceFlinger màn theo sau dùng **lịch riêng của chúng** sau thời điểm present dự kiến của pacesetter, với TODO
mở b/256196556 "use the next VSYNC after the frontrunner, not the pacesetter" (`Scheduler.cpp:217-238`) [V, Codex]. Đường
vẽ macOS của GPUI chạy trên main thread, kể cả callback của subscriber CVDisplayLink trên main queue
(`gpui_macos/src/display_link.rs:359`, `window.rs:2202`) [V, Codex], nên frame dài của cửa sổ này chặn cửa sổ kia; link
theo màn sửa nhịp, không sửa can nhiễu. Nguồn vsync native **có**, nhưng ngữ nghĩa và mức hỗ trợ khác nhau (bản trước viết
"không có": sai [Codex]): macOS có race vòng đời CVDisplayLink, `NSView.displayLink` từ macOS 14 bám màn của view và Apple
nói rõ không gọi callback khi view ẩn hoặc không ở trên màn nào; Windows có DXGI frame statistics và thống kê theo target ở
Windows 11; Wayland có frame callback và presentation feedback. winit 0.30.13 không có sự kiện vsync, đổi màn hay cắm rút
riêng; `Moved` không hỗ trợ trên Wayland [V, Codex]. Compositor không thống nhất cửa sổ trải hai màn theo nhịp nào (diện
tích lớn nhất, giữ output cũ, hay nhanh nhất).

### 11.2 Code của ta hôm nay

- **Một cửa sổ** [V, tôi đã kiểm]: `Platform.state: Option<WindowState>` (`window.rs:656`); `resumed` tạo một cửa sổ;
  `window_event` bỏ qua id; `app::run` mở một cửa sổ. Review đã ghi chưa kiểm (`docs/review-2026-11-23.md:1028`).
- **Thread mỗi cửa sổ:** `spawn` tạo một thread "gui render" và một "gui ui" mỗi cửa sổ (`window.rs:567-638`);
  `GpuRenderer::for_window` xin adapter và tạo device/queue riêng (có thể cùng GPU vật lý); chỉ `wgpu::Instance` là theo
  process [V, Codex]. Mở rộng ngây thơ thành N UI thread, N render thread, N device [I]. Arena reactive là của thread hiện
  tại (`reactive/runtime.rs:406`), nên hai cửa sổ trên hai UI thread không dùng chung signal được [V, Codex]. Bản trước
  viết "một UI thread chỉ giữ được một frame time": **sai**, `FRAME_TIME` là một ô mỗi thread nhưng `begin_frame` ghi đè nó
  (`clock.rs:148`), nên một UI thread chung phục vụ lần lượt các cửa sổ với frame time khác nhau được; việc ngoài frame cần
  một ngữ cảnh thời gian được định nghĩa [Codex].
- **Ước lượng margin** theo `RenderThread`: chi phí tệ nhất trong tối đa 32 mẫu cộng slack, kẹp [3 ms, max(3 ms,
  interval/2)] (`thread.rs:528, 638`) [V, Codex sửa cận].
- **Đổi màn** [V đường code, tôi đã kiểm]: interval chỉ được đọc lại trong `resized()`, chạy khi `Resized` hoặc
  `ScaleFactorChanged` (`window.rs:925, 1061`), qua `current_monitor().refresh_rate_millihertz()` (`window.rs:771`).
  `WindowEvent::Moved` không được xử lý. Dời giữa hai màn **cùng scale, khác Hz** giữ interval cũ [V, Codex; delegate
  macOS của winit phát `Moved`, handler đổi màn chỉ chỉnh simple fullscreen; cần xác nhận trên phần cứng]. Trên Wayland,
  `current_monitor()` lấy output đầu tiên trong iterator của winit, không nhất thiết là output compositor dùng cho
  presentation [Codex]. `Vsync::set_interval` bỏ phép khớp pha (`clock.rs:325-330`) [V]. Bản trước viết "học lại tốn tới 16
  lần acquire" và "khoảng một nửa `present_at` sớm 8,3 ms": **cả hai sai** [Codex]. Mẫu nhận đầu tiên đã đặt pha, 16 chỉ
  là dung lượng lịch sử; thời gian hội tụ không có cận khi thiếu bằng chứng hợp lệ. Lưới 120 Hz cũ có một tick thừa giữa
  hai vblank 60 Hz; nếu mọi acquire trả về đúng vblank thật thì dự đoán sau acquire có thể chọn tick thừa đó **mọi frame**,
  sớm khoảng 8,33 ms. Lần thức timer khi acquire thất bại cũng có thể lọt vào phép khớp như mẫu
  (`compositor/thread.rs:479, 558`).
- **Scale:** platform gửi `ToRender::Resize` và `ToUi::Resized` thành hai thông điệp riêng (`window.rs:925-934`); render
  thread cấu hình lại kích thước mới và vẽ scene cũ trước commit của UI: **có thể lệch nhiều frame**, thời lượng chưa đo
  [V mức code, Codex sửa "một frame"].
- **Màu:** renderer chọn định dạng 8 bit và `color_space: Auto` một lần; không gì phía app cập nhật không gian màu hay
  headroom theo màn [V]. **Cắm rút, gập máy:** không xử lý riêng; surface lost/outdated và dựng lại device cấu hình lại
  bằng config đã lưu (`gpu/mod.rs:575, 1119`) [V, Codex].
- **Cửa sổ trải hai màn:** interval danh nghĩa lấy từ `current_monitor` của winit, pha lấy từ lúc OS nhả
  `get_current_texture`, theo lựa chọn màn của compositor; hai cái có thể lệch nhau [I].

### 11.3 Ta có gì mà họ không có

1. **Đã có render thread riêng mỗi surface, tách khỏi UI thread:** acquire chặn trên màn A không chạm thread của cửa sổ B,
   và frame compositor của B (cuộn, fling, animation compositor) vẫn chạy khi UI thread chung đang bận. GPUI trên macOS
   không làm được điều này. Giới hạn [Codex]: tách thread chỉ cô lập việc chờ CPU; present cùng device vẫn chặn cả hai
   cửa sổ (wgpu 30.0.2), và device riêng vẫn chung GPU, driver, compositor. Hiện có hai `Offscreen` là hai UI thread
   riêng, chưa phải một UI thread chung.
2. **Biết đường cong animation** (10.4): biết tần số cần của mỗi cửa sổ, nên cửa sổ trên màn 144 Hz không phải chạy 144 Hz.
3. **Harness replay tất định với đồng hồ màn giả** 60/120/144 Hz (`compositor/replay.rs`) và `Pace::Ticker` cho thread
   headless: mở rộng thành nhiều màn là việc trực tiếp.
4. **Sở hữu cấu hình surface và blit cuối**, nơi đặt phép biến đổi đầu ra theo màn (9.5 D).

### 11.4 Thiết kế đề xuất

| Ý | Đánh giá | Lý do |
|---|---|---|
| **A. `DisplayClock` mỗi màn, cửa sổ đăng ký vào.** Nguồn thời gian phải **định danh output hoặc surface** và ghi mỗi mốc là dự đoán, đã present hay tổng hợp. Telemetry acquire giữ riêng theo surface, **không gộp** giữa các cửa sổ [Codex: gộp trộn backpressure driver, độ sâu hàng đợi, trễ lập lịch và có thể khác output; mẫu sạch vẫn có thể nhầm 60 Hz với mỗi tick thứ hai của 120 Hz]. Ưu tiên presentation feedback, DXGI frame statistics, display link | Thật | Cửa sổ dời màn có pha "ấm" ngay khi biết nguồn mới. Không mới: GPUI #60696 có registry theo màn; SurfaceFlinger khớp mốc vsync phần cứng và present fence cùng thông tin mode |
| **B. Khớp cả chu kỳ, không chỉ pha;** `refresh_rate_millihertz` chỉ là prior. Với VRR/ProMotion, biểu diễn **cơ hội present và cận khoảng**, không một chu kỳ cố định. Topology lấy **độc lập** qua thông báo native (`NSApplicationDidChangeScreenParametersNotification`), sự kiện output Wayland hoặc liệt kê monitor định kỳ, vì cửa sổ đứng yên hay nghỉ không sinh mẫu [Codex] | Thật, có ích cả cho một cửa sổ | Bản trước gọi khớp chu kỳ là "bộ dò cắm rút duy nhất": sai, có nguồn topology native |
| **C. Một UI thread cho mọi cửa sổ, một render thread mỗi cửa sổ.** UI thread phục vụ việc sẵn sàng theo deadline sớm nhất; frame time và frame request theo cửa sổ; định nghĩa công bằng cho timer và mailbox | Thật; cần để có nhiều cửa sổ | Deadline sớm nhất **không ngắt được** một layout 30 ms đang chạy [Codex]. Frame compositor của B giữ nhịp khi A trễ, trừ khi present cùng device chặn. Chromium là tiền lệ cho việc compositor sống sót khi UI trễ, nhưng bản trước viết "một thread compositor mỗi cửa sổ native" là **sai**: UI browser dùng layer-tree host đơn luồng và luồng Viz chung [Codex] |
| **D. Device dùng chung, submit theo cửa sổ** | Thật, chỉ khi đo thấy an toàn | Bỏ atlas và pipeline trùng, upload ảnh dùng chung một lần. Với wgpu 30.0.2, **không thể** bảo đảm cô lập vô điều kiện trên device chung khi present chặn [Codex]. Device riêng bỏ được khoá này nhưng vẫn chung GPU, driver, compositor. Quyết định bằng thí nghiệm |
| **E. Thế hệ surface (`SurfaceGeneration`).** Số đơn điệu; bỏ thông điệp và commit cũ. Đổi nguồn thời gian **ngay khi biết nguồn mới**, không chờ UI. Chốt cùng lúc kích thước buffer, scale, mã hoá đầu ra và scene khi buffer khớp sẵn sàng. Định nghĩa hành vi buffer giữ lại và phục hồi khi resize native, surface bị invalidate, màn bị rút. Mang thế hệ vào snapshot hiển thị và toạ độ input. Đọc lại khi `Moved`, `ScaleFactorChanged`, `Occluded`, `Focused`, thông báo topology, và khi chu kỳ khớp lệch | Thật | Sửa race Resize/Resized tách đôi, interval cũ, không gian màu cố định. Bản trước bắt chờ UI nhận đồng hồ mới thì mục tiêu "≤ 1 frame" không đạt được khi UI đang chạy việc dài [Codex]. Tiền lệ: `LocalSurfaceId`/`SetScaleAndSize` của Chromium, không bảo đảm OS giãn buffer cũ |
| **F. Replay tất định nhiều màn.** N đồng hồ màn giả, pha độc lập; dòng thời gian có kịch bản (dời ở t, đổi tần số, cắm rút, gập máy, feedback rơi, thế hệ cũ, trễ UI/acquire/present tiêm vào); khẳng định: target đủ điều kiện đầu tiên trên đồng hồ mới **sau một sự kiện retarget có thẩm quyền**, và B giữ nguyên các target compositor đủ điều kiện khi A trễ [Codex] | Thật | Nối với mục 4 và ADR-0023; kiểm logic lập lịch và render trên lavapipe offscreen. Nhịp thật, vblank lỡ, chuyển scale và đứng chéo cửa sổ cần feedback native hoặc trace độc lập: `FrameStat.present_at` hôm nay là **dự đoán** (`thread.rs:649`) |
| **G. Vsync giả headless trong `Offscreen` với hai ticker 60 và 144 Hz** | Thật (mức thread) | Kiểm dispatch giữa thread thật; lưu ý hai `Offscreen` hiện là hai UI thread riêng |
| Render thread mỗi màn thay vì mỗi cửa sổ | Loại lúc này | Acquire chặn gây head-of-line; xét lại khi có nguồn sẵn sàng không chặn, với điều kiện cô lập submit/present |
| Vẽ tách cửa sổ trải hai màn theo từng màn | Mánh | Một swapchain có một mã hoá do app chọn; OS vẫn có thể compose nó lên nhiều output với đồng hồ và biến đổi màu khác nhau; đã loại ở 9.5 J |
| Device GPU theo từng màn (đa adapter) | Tuỳ topology | Copy chéo adapter xuất hiện khi GPU sản xuất và GPU hiển thị khác nhau; dời cửa sổ có thể làm điều đó xảy ra [Codex] |

**ADR đổi:** 0015 (1 platform + 1 UI + N render thread; `DisplayClock`; khớp chu kỳ; epoch di chuyển; thứ tự deadline trên
UI thread), 0011 (chính sách dùng chung device; cấu hình lại surface theo epoch), 0013 (`FRAME_TIME` và hàng đợi timer
theo cửa sổ), 0023 (replay nhiều màn là một trục kiểm chứng), ADR màu dự kiến ở mục 9 (phép biến đổi đầu ra chọn theo
epoch), và một ADR mới "cửa sổ" cho API nhiều cửa sổ, stack layer và focus theo cửa sổ. Số ADR mới **không được là
0024**: đã có `docs/adr/0024-the-engine-keeps-the-layer-stack-and-focus-scopes.md` [Codex]; ADR mới mở rộng 0024.

**Phạm vi của "có thể mới"** (đã qua kiểm chứng Codex, thu hẹp thêm). Tiền lệ đáng kể: Chromium mock display link và test
retarget/reconnect, khoảng được chọn và giao begin-frame (`external_begin_frame_source_mac_unittest.cc`); PR Flutter
#196776 (chưa merge) test chuyển tần số và pha feedback; SurfaceFlinger viết kịch bản tín hiệu frame nhiều màn, commit bị
bỏ, đổi pacesetter, chuyển tần số VRR trên đồng hồ giả và test gập/mở (`SchedulerTest.cpp:518`,
`SurfaceFlinger_FoldableTest.cpp`); KWin test thêm/bớt output và dời cửa sổ (`outputchanges_test.cpp`); Mutter test gập
máy/cắm rút và chạy frame clock độc lập theo view; Weston test presentation feedback và rút output; GPUI #67295 đo bằng CI
các lần đổi view có kịch bản; Godot Movie Maker và VNCplay phủ các mảnh lân cận. Registry display link theo màn (GPUI,
Chromium), render loop theo output (KWin), resize/scale đồng bộ (Chromium) đều không mới. Điều có thể còn mới [Codex: không
kiểm được là mới, chỉ là chưa tìm thấy]: **một test toolkit desktop gộp hai đồng hồ màn độc lập có kịch bản, dời cửa sổ
cộng đổi mode/cắm rút, khẳng định frame đủ điều kiện đầu tiên sau retarget, và khẳng định trễ UI tiêm vào cửa sổ A không
đổi target compositor của cửa sổ B.**

### 11.5 Thí nghiệm đầu tiên và rủi ro

**Thí nghiệm "hai nhịp, một process", hai giai đoạn.**

*Giai đoạn 1, code hiện tại, một cửa sổ, MacBook ProMotion (120 Hz) cộng màn rời 60 Hz cùng scale.* Ghi `FrameStat` khi kéo
cửa sổ qua màn. Đo: `ScaleFactorChanged` hay `Resized` có bắn không; sai số `present_at` so với nhịp acquire quan sát được,
trước và sau khi dời; số frame tới khi khoảng frame ổn định. Dự đoán (đã sửa theo Codex): interval giữ giá trị cũ, và
`present_at` có thể sớm khoảng 8,33 ms ở **mọi** frame chứ không phải một nửa. `present_at` là dự đoán, nên cần
presentation feedback native hoặc trace độc lập (Instruments, PresentMon) làm đối chứng.

*Giai đoạn 2, prototype C và D: hai cửa sổ trên hai màn.* Cửa sổ A chạy layout giả 30 ms mỗi frame; B chạy spinner và
fling. Đo ở chế độ device chung và device riêng, trên Metal và trên Windows NVIDIA Vulkan.

| Chỉ số | Mục tiêu |
|---|---|
| Khoảng frame của B, p50/p99, so với chu kỳ màn của nó | p99 ≤ chu kỳ + 1 ms |
| Vblank B lỡ mỗi giây | 0 |
| Lần đứng submit/present lâu nhất của B do A gây ra | ≤ 0,5 ms (đạt/không đạt cho device chung) |
| Dời cửa sổ từ 120 sang 60 Hz: số frame tới khi khoảng frame bằng chu kỳ mới | ≤ 1 với A và E |
| Frame hiện nội dung scale cũ sau khi dời 1× → 2× | 0 |
| Bộ nhớ: device chung so với device riêng | báo cáo |

**Rủi ro:**
- Pha từ acquire chặn: wgpu#9019 nói acquire dưới FIFO không phải tín hiệu vblank tin được; dưới VRR hay ProMotion khoảng
  cách không phải chu kỳ cố định, nên khớp chu kỳ có thể đuổi theo tần số nội dung thay vì của màn [V, Codex].
- winit không có sự kiện đổi màn; cần thông báo topology native hoặc liệt kê định kỳ, không chỉ khớp chu kỳ.
- Present bị chặn trên device chung có thể tuỳ driver; ma trận test phải có NVIDIA Vulkan.
- Một UI thread nghĩa là frame dài ở A làm trễ main frame của B (không phải frame compositor); thứ tự deadline cần ưu
  tiên input.
- N render thread ở ưu tiên thời gian thực cố định (10.3) có thể bỏ đói hệ thống khi nhiều cửa sổ; giới hạn số lượng
  hoặc hạ ưu tiên cho cửa sổ không focus.
- Occlusion của macOS có thể sai (zed#67004, #63217 là báo cáo khả dĩ, chưa tái hiện ở shadcn-rs); cổng `Visible(false)`
  của ta (`thread.rs:321`; Wayland còn chặn theo callback) có thể đóng băng tương tự, cần đường kiểm lại/phục hồi; ưu tiên
  cao phải tính trên tổng số worker đang chạy [Codex].
- Display link thật cần `unsafe` hoặc việc ở upstream (`unsafe_code = "forbid"`).

---

## 12. Trạng thái

Cả chín bài đã nghiên cứu xong và cả chín (mục 3–11) đã qua kiểm chứng độc lập của Codex; các chỗ sửa đánh dấu
"[V, Codex]" hoặc "[Codex]", câu sai cũ được nêu lại thay vì xoá.

## 13. Quyết định

**Đã chốt (2026-11-23):** không tương thích ngược, kể cả cho bản ghi replay. Định dạng đi theo code, bản ghi trong repo
được chuyển cùng commit (4.4).

**Không cần quyết định, là lỗi cần sửa** (đã qua kiểm chứng của Codex):
- Cache layout chấp nhận nhầm một cặp khoá va chạm dưới điều kiện cụ thể (6.1); chưa có vết widget thật.
- `set_estimate` O(số run) trên đường cuộn (7.3 #1).
- Ước lượng trung bình cũ khi chiều cao dòng đã đo thay đổi (7.3 #1).
- Chiều cao ngoài màn hình gắn nhầm dòng sau sort cùng độ dài (7.3 #5).
- Kết quả sort của `View` có thể về sai thứ tự trên cùng snapshot (4.1).
- State toàn process ngoài registry Fork (mục 2).
- `text-wrap: pretty` không làm gì trên đoạn hơn 4 dòng (8.2); Codex xác nhận code sai, ADR-0005 đúng.
- Khi có animation, mỗi vòng `Surface` đủ điều kiện blit và present toàn cửa sổ kể cả khi damage rỗng, vì acquire để lấy
  nhịp vsync làm `holds_frame()` thành true (10.3); trái với chú thích ở `gpu/mod.rs:20-25` và kỳ vọng của ADR-0016.
- Caret không bao giờ dừng nhấp nháy khi còn focus; `input_otp` giữ render thread vẽ mỗi vblank mãi mãi (10.3).
- `PowerPreference` là `None`, nên trên máy hai GPU có thể chọn GPU rời (10.3).
- Dời cửa sổ giữa hai màn cùng scale, khác Hz giữ interval cũ (`Moved` bị bỏ qua); khi đó `present_at` có thể sớm khoảng
  8,33 ms ở mọi frame (11.2).
- Lần thức timer khi acquire thất bại có thể lọt vào phép khớp pha như một mẫu vsync (11.2).
- `ToRender::Resize` và `ToUi::Resized` tách đôi: renderer vẽ scene scale cũ vào kích thước mới, có thể nhiều frame (11.2).
- Không gian màu và headroom chọn một lần, không theo màn; không xử lý riêng cắm rút và gập máy (11.2).
- Một cửa sổ duy nhất là giới hạn kiến trúc hiện tại, cần ADR mở rộng ADR-0024 (11.4).

**Giới hạn có tài liệu, không phải lỗi:** scatter yêu cầu x không giảm (7.3 #10); một ước lượng toàn cục (7.3 #7).

**Đã chốt thêm (2026-11-23, người dùng: "ta cần điều tốt nhất"):** cả ba câu hỏi chọn phương án đầy đủ, ở hình dạng cuối.

1. **Frame rẽ nhánh có trong bản release, kèm outbox** (mục 2). Một cơ chế duy nhất cho debug và release, không có nhánh
   chỉ-debug. Undo log chỉ ghi khi có nhánh đang mở, nên frame thường không trả phí. Mọi tác dụng ra ngoài (mailbox,
   spawn, clipboard, I/O qua handle app cấp, hành động accessibility) đi qua outbox có kiểu: commit thì gửi, rollback thì
   bỏ; API tác dụng chỉ lấy được qua outbox, nên không có đường vòng. State ngoài registry được đưa vào registry
   (`ListState`, bộ đếm id series/page) hoặc được chứng minh và ghi rõ là cache đơn điệu không ảnh hưởng ngữ nghĩa (style
   interner, theo nội dung). Đổi ADR-0013, ADR mới.
2. **Nguồn thông tin màn hình của hệ điều hành qua một crate ranh giới riêng** (3.7, 10.4, 11.4, review mục 18). Mọi
   `unsafe` của OS nằm trong **một** crate nhỏ (vsync/presentation feedback, topology và cắm rút, không gian màu và
   headroom HDR theo màn), lộ ra API an toàn: sự kiện theo màn có định danh output, mốc thời gian ghi rõ là dự đoán, đã
   present hay tổng hợp, và khoảng tần số cho VRR. `gui` và mọi crate khác giữ `forbid(unsafe_code)`. Riêng crate đó:
   `unsafe_code` chỉ cho phép theo từng chỗ, `clippy::undocumented_unsafe_blocks` và
   `clippy::multiple_unsafe_ops_per_block` ở mức deny, mỗi khối có `// SAFETY:`, mỗi lần đổi qua review Codex. Một ranh
   giới phục vụ cả mục 3, 9, 10, 11 thay vì mỗi mục một vá. ADR mới.
3. **MCP server có sẵn trong mọi app** (5.5), an toàn mặc định. Biên dịch sẵn trong mọi app để nhà phát triển không phải
   nhớ bật; nhưng lúc chạy không lắng nghe gì cho tới khi **người dùng cuối** cấp quyền cho từng agent. Chỉ transport cục
   bộ: socket Unix trong thư mục quyền 0700 của user, named pipe có ACL theo SID của user trên Windows; không TCP. Quyền
   theo agent và theo intent; intent rủi ro cần xác nhận của người trên một layer mà agent không với tới được
   (ADR-0024). Đổi ADR-0010, ADR mới "App được vận hành qua intent có kiểu".

Thêm: ADR "cửa sổ" (11.4) mở rộng ADR-0024; số ADR mới bắt đầu từ 0026 (0024 và 0025 đã có).
