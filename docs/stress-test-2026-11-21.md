# shadcn-rs — Báo cáo stress test & devtools CLI

2026-11-21. Bản trên Claude Docs: https://claude.ai/artifact/VtDu2eMd4vjPJqi6sCwCNA

## Tóm tắt

Kết quả: không panic, không lỗi layout, không leak trên 10 kịch bản stress và cả 85 page gallery. Hai lỗi thật trong
`gui` đã được sửa và có test; hai vấn đề hiệu năng còn mở.

- **Phạm vi:** 10 kịch bản stress mới (đến 60 000 node, 5 000 signal, 5 000 message từ thread khác) và `monkey` 80
  bước ngẫu nhiên trên mỗi page trong 85 page.
- **Đã sửa:** pool layer cỡ cửa sổ chỉ tăng không giảm (giữ thêm 9.4 MB mãi mãi); text vẽ cả những dòng đã bị cắt khỏi
  vùng cuộn (paint của kịch bản `text` giảm khoảng 42%).
- **Còn mở:** mỗi frame build và layout lại toàn bộ cây (60 000 node mất khoảng 360 ms một frame); một phần tử nhỏ có
  opacity hoặc bị xoay vẫn chiếm một layer cỡ cửa sổ.
- **Kiểm tra:** clippy sạch, 46 test pass, ảnh render giống nhau từng byte trước và sau khi sửa.

## Công cụ và cách chạy

Mọi phép đo đều đi qua devtools CLI (`gui::devtools`, ADR-0010): cùng một bộ lệnh chạy headless trên `Scenario` (đồng
hồ được script hoá) hoặc gắn vào cửa sổ đang chạy qua Unix socket `$TMPDIR/gui-devtools-<pid>.sock`.

**Example `stress`** (`crates/shadcn/examples/stress/`):

```sh
cargo run --release -p shadcn --example stress -- --list
cargo run --release -p shadcn --example stress -- --run [scene…] [--cycles=5 --monkey=150 --budget=16 --growth-kb=64]
cargo run --release -p shadcn --example stress -- --inspect table -e 'perf' -e 'memory'
cargo run --release -p shadcn --example stress -- --window grid   # có HUD và socket devtools
```

`--run` đưa mỗi kịch bản qua bốn bước:

1. **Script**: các thao tác của kịch bản; mỗi thao tác được tính thời gian theo frame nó gây ra, in ra thao tác chậm
   nhất.
2. **Steady**: `bench 30`, p50/p95 của tổng, build, layout, paint, và footprint (node, heap, ảnh cache).
3. **Cycle**: lặp lại một vòng thao tác (mặc định 5 lần). Baseline heap lấy sau vòng warm-up thứ hai (font, entry cache
   đầu tiên); FAIL nếu mỗi vòng sau giữ lại hơn 64 KB. `forget_log` loại bộ nhớ của log devtools khỏi phép đo.
4. **Monkey**: `monkey N --seed=7`, thao tác ngẫu nhiên (click, hover, wheel, drag, type, phím) có `catch_unwind` quanh
   mỗi bước; báo panic, layout lỗi, frame chậm và heap tăng.

Kịch bản FAIL khi một lệnh lỗi, monkey gặp panic hoặc layout lỗi, hoặc một cycle giữ bộ nhớ. Frame chậm chỉ được báo,
không FAIL: các kịch bản được dựng để chậm.

**Quét gallery**: mỗi page của docs example chạy `--inspect <page> -e 'monkey 80 --seed=3'`, exit code khác 0 là lỗi.

## 10 kịch bản stress

Cả 10 kịch bản PASS: script chạy hết, monkey 150 bước không gặp panic hay layout lỗi, và không cycle nào giữ quá 1.4 KB
(ngưỡng là 64 KB). Ba kịch bản nhiều node vượt ngân sách 16 ms một frame.

| Kịch bản | Thử gì | Frame p50 (steady) | Giữ lại mỗi cycle |
|---|---|---:|---:|
| table | bảng 5 000 dòng, layout toàn bộ: lọc, sắp xếp, chọn | **362 ms** | 0 B |
| grid | 2 500 nút có style hover, mỗi nút đếm click riêng | **80 ms** | 0 B |
| churn | 300 card mount và unmount, mỗi card có state riêng | **42 ms** | 0 B |
| text | 60 KB text năm hệ chữ, một từ 2 000 ký tự, input 5 000 ký tự | 15.6 ms | 1.4 KB |
| shuffle | 400 dòng có key: xáo, đảo, chèn, xoá | 9 ms | 0 B |
| dashboard | app admin: navigation, tabs, KPI, bảng khách hàng, dialog sửa, toast | 5.9 ms | 0 B |
| tree | cây file 1 600 mục sâu 4 cấp, chuỗi 120 box lồng nhau | 4.4 ms | 0 B |
| feed | một thread gửi 5 000 message qua mailbox, list giữ 200 cái cuối | 3 ms | 0 B |
| reactive | 5 000 signal, chuỗi 2 000 computed, 500 effect | 0.5 ms | 284 B |
| toasts | 30 toast cùng lúc, timer, hover để mở rộng, animation thoát | 0.15 ms | 1 B |

Số node quyết định thời gian frame. Bảng 5 000 dòng (60 000 node) dành 334 ms cho layout, giữ 252 MB heap, và một lần
hover mất khoảng 600 ms vì cả cây được build và layout lại. Reactive (5 000 signal, 2 000 computed) chỉ tốn 0.5 ms:
graph reactive không phải nút thắt.

## Quét monkey 85 page

Cả 85 page của docs gallery đều exit 0 với `monkey 80 --seed=3`: không panic, không layout lỗi.

- **Frame chậm nhất (max)**: sheet 121 ms, form_vee 94 ms, dialog 93 ms, sidebar 92 ms (71 frame vượt ngân sách),
  command 82 ms.
- **Heap**: lần quét đầu, gần như mọi page tăng +9.4 MB sau 80 bước. Nguyên nhân là pool layer cỡ cửa sổ (xem phần lỗi
  đã sửa). Sau khi sửa, page `button` từ +9.4 MB còn −7 KB.
- **Còn +10 MB** trên sheet, context-menu, navigation-menu và sonner: monkey kết thúc khi một menu hoặc sheet còn mở,
  nên layer vẫn đang được dùng, không phải leak. Nhưng nó lộ ra một vấn đề khác: một phần tử nhỏ vẫn giữ cả một layer
  cỡ cửa sổ (item bị disable ở opacity 0.5, nút đóng ở opacity 0.7, chevron xoay 180°). Xem phần vấn đề còn mở.

## Lỗi gui đã sửa

### 1. Pool layer chỉ tăng, không giảm

- **Triệu chứng:** sau khi một group có opacity hoặc transform được vẽ một lần (hover, mở menu), process giữ thêm
  khoảng 9.4 MB mãi mãi, kể cả khi không còn group nào được vẽ.
- **Nguyên nhân:** `Painter` (`crates/gui/src/paint.rs`) giữ một pool layer cỡ cửa sổ để dùng lại, nhưng không bao giờ
  trả lại.
- **Sửa:** Painter ghi số layer dùng nhiều nhất trong frame này và frame trước (`layers_peak`), rồi cắt pool về mức lớn
  hơn trong hai frame, giống quy tắc "hai frame cuối" của cache (ADR-0008). Một frame không có group vẫn giữ layer cho
  frame sau; hai frame liền không có thì layer được trả.
- **Test:** `group_layers_are_given_back_once_no_group_paints` trong `app.rs`.

### 2. Text vẽ cả dòng nằm ngoài vùng nhìn thấy

- **Triệu chứng:** kịch bản `text` (60 KB trong một vùng cuộn) mất ~25 ms mỗi frame cho paint, dù chỉ vài chục dòng
  hiện trên màn hình.
- **Nguyên nhân:** `text.rs` vẽ mọi dòng của `layout_runs()`; clip chỉ bỏ pixel sau khi đã rasterize glyph.
- **Sửa:** `text::Target` có thêm `visible: Option<Bounds>` (giao của clip cha và clip của node), và `draw()` bỏ qua
  dòng nào nằm hẳn ngoài vùng đó (có nới một line-height mỗi phía cho dấu và nét chữ tràn).
- **Kết quả:** paint của kịch bản `text` (p50/p95) từ 24.9/27.0 ms xuống 14.3/15.2 ms, giảm 42%.
- **Kiểm chứng:** 81 frame trên 13 page và 21 frame của docs trace giống nhau từng byte trước và sau khi sửa.

## Cải tiến devtools

Viết script cho các kịch bản là cách dùng thử devtools như một agent sẽ dùng. Những chỗ vấp đã được sửa trong devtools,
không phải trong script:

- **`monkey N [--seed=S --budget=MS]`** (mới): thao tác ngẫu nhiên có seed (xorshift), `catch_unwind` quanh mỗi bước,
  báo panic kèm bước và seed để chạy lại, layout lỗi, frame vượt ngân sách và heap tăng.
- **Tự cuộn tới node**: click, hover, drag trên một node nằm ngoài vùng cuộn sẽ `reveal` nó trước, như một người cuộn
  tới rồi bấm. Trước đó click rơi vào điểm ngoài màn hình.
- **Selector `#17`**: không có node tên `17` thì tìm theo text, nên `click #17` bấm được dòng có chữ "#17".
- **`setvalue` / `activate` tìm đúng node**: selector trúng label thì đi tới node AccessKit có action đó (`actor()`),
  thay vì báo lỗi.
- **`group(...)` trong `node`**: in lý do một node được vẽ vào layer riêng (opacity, transform, clip bo góc). Nhờ đó
  tìm ra các layer cỡ cửa sổ của phần tử nhỏ.
- **`Session::forget_log`**: bỏ log 120 frame của devtools trước khi đo heap, để bộ nhớ của chính công cụ đo không bị
  tính là leak.

## Vấn đề còn mở

### Build và layout lại toàn bộ cây mỗi frame

Mỗi frame gọi lại render của cả app và layout mọi node, kể cả node nằm ngoài màn hình. Với 60 000 node (bảng 5 000
dòng) một frame mất ~360 ms, một lần hover ~600 ms. App thật có list dài sẽ gặp điều này trước.

- **Hướng sửa:** một list ảo (virtual list) chỉ build các dòng nằm trong vùng nhìn thấy cộng một ít dự phòng; về lâu
  dài, layout tăng dần (chỉ tính lại nhánh có thay đổi).
- **Đo lại bằng:** `stress --run table grid churn`.

### Phần tử nhỏ chiếm layer cỡ cửa sổ

Một group có opacity hoặc transform được vẽ vào một layer bằng cả cửa sổ (khoảng 9.4 MB ở kích thước test), dù nó chỉ
là một icon chevron. Pool đã trả layer khi không dùng, nhưng khi menu mở, mỗi group nhỏ vẫn tốn một layer.

- **Hướng sửa:** cấp layer theo vùng group vẽ (bounds sau transform), hoặc bỏ layer khi không cần: opacity trên một
  node không có con chồng nhau có thể nhân thẳng vào alpha, và xoay 90°/180° có thể vẽ trực tiếp.
- **Đo lại bằng:** `--inspect context_menu -e 'rclick …' -e memory`, hoặc quét monkey trên sheet, context-menu,
  navigation-menu, sonner.

## Trạng thái code

- **Nơi có code:** worktree `../shadcn-rs-stress`, detached tại `c4fb22f`, chưa commit. Thư mục
  chính chỉ có bản devtools trước đợt stress test.
- **Thay đổi:** example `stress` mới (`main.rs`, `scenes.rs`); `gui` sửa `paint.rs` (pool layer), `text.rs` (culling),
  `devtools.rs` (monkey, reveal, selector, actor, group, forget_log); ADR-0010 thêm mục "Found by the stress scenes";
  hai README thêm phần devtools và stress.
- **Kiểm tra cuối:** `cargo clippy` 0 warning, 46 test của `gui` pass, `cargo fmt` đã chạy; `stress --run` tất cả
  PASS; quét 85 page exit 0.
