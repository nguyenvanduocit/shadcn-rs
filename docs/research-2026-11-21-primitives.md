# shadcn-rs — Nghiên cứu: hành vi mà `primitives` phải giữ

2026-11-21. Đầu vào cho S6 của M10 ([ADR-0022](adr/0022-a-style-is-a-typed-value-components-read.md),
[kế hoạch](plan-2026-11-21.md)). Tài liệu ghi lại những gì reka-ui bảo đảm cho người dùng và cho công nghệ hỗ trợ, đọc
từ nguồn. Mục đích là để `primitives` giữ đúng giá trị đó trên hạ tầng của Rust và `gui`, không phải để chép reka.

## Mục lục

1. [Cách đọc](#1-cách-đọc)
2. [Tổng quan](#2-tổng-quan)
3. [Hạ tầng: Rust và `gui` thay cơ chế DOM](#3-hạ-tầng-rust-và-gui-thay-cơ-chế-dom)
4. [S6 cần quyết định](#4-s6-cần-quyết-định)
5. [Nhóm 1: lớp nổi và focus](#5-nhóm-1-lớp-nổi-và-focus): DismissableLayer, Phát hiện tương tác bên ngoài, FocusScope, FocusGuards, Presence, Teleport, Popper, Khoá cuộn và trơ con trỏ, Ẩn phần còn lại khỏi AT, Vùng an toàn của con trỏ, Chế độ nhập: bàn phím hay con trỏ, Liên kết ID trigger ↔ nội dung
6. [Nhóm 2: tập hợp, điều hướng, controlled state và form](#6-nhóm-2-tập-hợp-điều-hướng-controlled-state-và-form): RovingFocus, Collection, Primitive, VisuallyHidden, ConfigProvider, Label, Separator, useTypeahead, useArrowNavigation, useVModel, useSelectionBehavior, useFilter, useKbd, useDirection, useId, useForwardExpose, useForwardPropsEmits, useStateMachine, useSize, Tích hợp form
7. [Nhóm 3: overlay](#7-nhóm-3-overlay): Dialog, AlertDialog, Drawer, Popover, HoverCard, Tooltip và TooltipProvider, Toast
8. [Nhóm 4: menu](#8-nhóm-4-menu): Menu, DropdownMenu, ContextMenu, Menubar, NavigationMenu, Toolbar
9. [Nhóm 5: chọn và danh sách](#9-nhóm-5-chọn-và-danh-sách): Select, Combobox, Listbox, TagsInput, Autocomplete, Tree
10. [Nhóm 6: điều khiển form](#10-nhóm-6-điều-khiển-form): Checkbox, Switch, RadioGroup, Slider, Toggle, ToggleGroup, NumberField, PinInput, Progress, Stepper, `input_otp` của shadcn-rs
11. [Nhóm 7: ngày và giờ](#11-nhóm-7-ngày-và-giờ): Calendar, RangeCalendar, Tiện ích ngày dùng chung, TimeField, DateField, DatePicker, DateRangePicker
12. [Nhóm 8: cấu trúc](#12-nhóm-8-cấu-trúc): Accordion, Collapsible, Tabs, Pagination, ScrollArea, Splitter, Avatar, AspectRatio, Carousel
13. [Nguồn](#13-nguồn)

---

## 1. Cách đọc

- **Đọc để hiểu, không chép 1:1.** Khi duyệt việc này, người dùng nói: "đưa value qua, nhưng hạ tầng thì phải tuyệt đối
  tận dụng mọi lợi thế của rust". Từ reka ta lấy hai thứ:
  - điều người dùng và công nghệ hỗ trợ (AT) quan sát được;
  - con số hành vi: độ trễ, ngưỡng, bảng phím, luật giá trị.

  Ta không lấy hình dạng API của reka (tên prop, emit, `data-*`, biến CSS) và không lấy cơ chế DOM của nó.
- **Nhãn trong các mục của từng nhóm:**
  - **Bảo đảm**: điều người dùng hoặc AT quan sát được. S6 phải khớp, hoặc khác có chủ đích và ghi lại. Mục không có
    nhãn cũng là bảo đảm.
  - **Cơ chế DOM không port**: cách reka đạt bảo đảm trong trình duyệt, viết dạng `cơ chế → bảo đảm nó phục vụ`. S6 giữ
    bảo đảm, bỏ cơ chế. §3 ghi cách Rust và `gui` thay từng loại cơ chế.
  - **Look cần đọc**: trạng thái hoặc giá trị mà phần look của `shadcn` đọc từ primitive. Dòng này thay cho `data-*` và
    biến CSS của reka.
  - **Trạng thái**: state mà primitive sở hữu, ai được ghi (người dùng, caller), và mỗi lần ghi nghĩa là gì. "Caller"
    là code dùng primitive. "Người dùng" là người thao tác. "Phủ quyết" là caller huỷ hành động mặc định.
  - **Dễ sót**: bảo đảm mà một bản cài đặt dễ bỏ qua.
  - **Docs lệch**: docs của reka nói khác source. Source thắng.
  - **Bug reka?** và **Quirk**: reka làm vậy nhưng trái ý định của chính nó, hoặc không đáng chép. S6 làm theo ý định
    và ghi lý do.
  - **chưa kiểm**: không đọc được nguồn để chắc. **suy từ mã**: đọc được trong source, không có test khẳng định.
- **Khối dựng chung** nằm ở nhóm 1 và 2 (§5, §6): lớp nổi, focus, Presence, Popper, Collection, roving, typeahead,
  controlled state, form. Các nhóm sau chỉ ghi mỗi primitive cấu hình chúng thế nào.
- **Cách làm.**
  - Tám agent đọc song song, mỗi agent một nhóm, chỉ đọc file.
  - Mỗi agent tự đối chiếu trích dẫn với source trước khi nộp.
  - Sau đó 17 trích dẫn, hai tới ba mỗi nhóm, được kiểm lại trực tiếp trong source. Cả 17 đúng. Mọi con số ở §2.3 cũng
    được kiểm lại trong source.
  - Khi ghép, tiền tố trích dẫn và nhãn trạng thái của tám nhóm được gộp về một bộ (bảng dưới). Nội dung giữ nguyên.
- **Chỗ không kiểm được.** Các thư viện sau không có trong `references/`: floating-ui, gói `aria-hidden`, vueuse,
  `@internationalized/date`, `@internationalized/number`, TanStack Virtual. Hành vi dựa vào chúng chỉ suy từ cấu hình
  reka truyền vào và từ test của reka; các mục đó ghi rõ điều này.

Trích dẫn dùng một bộ tiền tố cho cả tài liệu. `.shadcn-vue` là shadcn-vue `d1bf1b3`, pin của verdict.

| Dạng | Tính từ |
| --- | --- |
| `path:line` | `references/reka-ui/packages/core/src/` (reka-ui 2.11.0, commit `f67a0e0`), kể cả file test |
| `docs:` | `references/reka-ui/docs/content/docs/` |
| `meta:` | `references/reka-ui/docs/content/meta/` |
| `shadcn:` | `crates/shadcn/verdict/.shadcn-vue/apps/v4/registry/bases/reka/ui/` |
| `shadcn-ex:` | `crates/shadcn/verdict/.shadcn-vue/apps/v4/registry/bases/reka/examples/` |
| `shadcn-style:` | `crates/shadcn/verdict/.shadcn-vue/apps/v4/registry/styles/` |
| `vue:` | `references/vuejs/packages/` |
| `vueuse:` | `@vueuse/core` và `@vueuse/shared` 15.0.0 từ một bản cài cục bộ, không có trong `references/` (reka ghim `^14.1.0`) |
| `crates/…` | gốc shadcn-rs |

## 2. Tổng quan

### 2.1 Component của shadcn và primitive của reka

Bảng đọc từ import `reka-ui` của từng component trong `.shadcn-vue/apps/v4/registry/bases/reka/ui/`.

| Component shadcn | Primitive reka | Nhóm |
| --- | --- | --- |
| `accordion` | Accordion | 8 |
| `alert-dialog` | AlertDialog | 3 |
| `aspect-ratio` | AspectRatio | 8 |
| `avatar` | Avatar | 8 |
| `calendar` | Calendar | 7 |
| `checkbox` | Checkbox | 6 |
| `collapsible` | Collapsible | 8 |
| `combobox` | Combobox | 5 |
| `command` | Listbox | 5 |
| `context-menu` | ContextMenu, trên engine Menu | 4 |
| `dialog`, `sheet` | Dialog | 3 |
| `drawer` | Drawer | 3 |
| `dropdown-menu` | DropdownMenu, trên engine Menu | 4 |
| `hover-card` | HoverCard | 3 |
| `label` | Label | 2 |
| `menubar` | Menubar, trên engine Menu | 4 |
| `navigation-menu` | NavigationMenu | 4 |
| `number-field` | NumberField | 6 |
| `pagination` | Pagination | 8 |
| `pin-input` | PinInput | 6 |
| `popover` | Popover | 3 |
| `progress` | Progress | 6 |
| `radio-group` | RadioGroup | 6 |
| `range-calendar` | RangeCalendar | 7 |
| `resizable` | Splitter | 8 |
| `scroll-area` | ScrollArea | 8 |
| `select` | Select | 5 |
| `separator` | Separator | 2 |
| `sidebar`, `tooltip` | Tooltip | 3 |
| `slider` | Slider | 6 |
| `stepper` | Stepper | 6 |
| `switch` | Switch | 6 |
| `tabs` | Tabs | 8 |
| `tags-input` | TagsInput | 5 |
| `toggle` | Toggle | 6 |
| `toggle-group` | ToggleGroup | 6 |

- **Không dùng reka:**
  - `carousel` dùng embla. Carousel của reka (nhóm 8) chỉ để tham khảo; verdict so với embla.
  - `sonner` dùng vue-sonner. Gần nhất trong reka là Toast (nhóm 3).
  - `input-otp` dùng vue-input-otp. Gần nhất trong reka là PinInput (nhóm 6).
  - `native-select` là thẻ gốc của trình duyệt.
  - `time_input` của `gui` ứng với TimeField của reka (nhóm 7).
- **Chỉ dùng `Primitive` hoặc `Slot`, không có hành vi:** `badge`, `breadcrumb`, `bubble`, `button`, `button-group`,
  `chart`, `code-block`, `item`, `marker`, `message`, `questionnaire`, `form`. Các component này không cần primitive.
- Không component nào gọi thẳng hạ tầng chung (nhóm 1, 2); các primitive ở bảng trên dùng nó.

### 2.2 Bảo đảm xuyên suốt

Các bảo đảm sau lặp lại ở nhiều nhóm. Chúng là lõi của `primitives` và của phần engine trong `gui`.

1. **Hiện diện, không phải tồn tại.**
   - Stack lớp, stack focus, ẩn nền khỏi AT, khoá cuộn và tham chiếu ARIA đều tính theo "đang hiện".
   - Nội dung giữ mount khi đóng thì rời mọi stack và ngừng mọi hiệu ứng phụ.
   - Nội dung đang chạy animation đóng vẫn hiện, nhưng state đọc là "đóng".
   - (nhóm 1, 3)
2. **Esc chỉ tới lớp trên cùng.** Tooltip mở trong dialog đóng trước dialog (nhóm 3).
3. **Bấm ra ngoài.**
   - Chuột và bút tính lúc nhấn xuống, với mọi nút. Chạm tính lúc tap xong; tap biến thành cuộn thì không tính.
   - Cú nhấn đã mở lớp không đóng nó.
   - Một cú nhấn ngoài mọi lớp đóng mọi lớp đủ điều kiện cùng lúc.
   - (nhóm 1)
4. **Một focus scope tạm dừng bẫy của scope bên dưới**, kể cả scope không bẫy. Nhờ vậy popup trong modal nhận được
   focus (nhóm 1).
5. **Mỗi lần primitive dời focus** đều chọn hết chữ của ô nhập và không cuộn trang (nhóm 1).
6. **Đóng thì trả focus** về phần tử có focus lúc mở (Dialog), dự phòng là Trigger. Không giành lại focus mà thứ khác
   đã lấy (nhóm 3).
7. **Tab vòng trong nội dung overlay**, kể cả Dialog, Popover và Drawer không modal (nhóm 3).
8. **Modal gồm bốn bảo đảm:** bẫy focus; AT chỉ thấy lớp đó; bên ngoài trơ với con trỏ; nền không cuộn.
   - Khi mở: focus vào trước, rồi mới ẩn nền khỏi AT.
   - Khi đóng: trả nền cho AT trước, rồi trả focus.
   - (nhóm 1, 3)
9. **Roving focus.**
   - Cả nhóm là một tab stop, và Shift+Tab thoát nhóm bằng một lần nhấn.
   - Vào nhóm bằng phím thì focus tới, theo thứ tự ưu tiên: item active, item highlight, tab stop hiện tại, item bật
     đầu tiên.
   - (nhóm 2)
10. **Typeahead.**
    - Chuỗi tìm tự xoá sau 1000 ms.
    - Gõ lặp một chữ thì xoay vòng qua các item bắt đầu bằng chữ đó.
    - Chuỗi dài hơn giữ item hiện tại nếu vẫn khớp.
    - Không phân biệt hoa thường.
    - (nhóm 2, 5)
11. **Thứ tự item là thứ tự hiển thị**, không phải thứ tự đăng ký. Với danh sách ảo hàng triệu dòng, các bảo đảm sau
    phải đúng trên toàn bộ dữ liệu: thứ tự, bỏ qua item tắt, typeahead, Home/End, chọn dải bằng Shift (nhóm 2, 5).
12. **Controlled hay uncontrolled quyết một lần khi tạo.**
    - "Caller không đưa" phải khác `false` hay rỗng.
    - Khi caller sở hữu, primitive chỉ đề xuất giá trị mới; state hiển thị chỉ đổi khi caller ghi lại.
    - (nhóm 2, 6)
13. **Menu.**
    - Highlight chính là focus; chuột rời item thì không còn item nào highlight.
    - Esc trong submenu đóng cả cây; mũi tên ngược chiều chỉ đóng một tầng.
    - Mở bằng phím thì highlight item đầu; mở bằng con trỏ thì không highlight item nào.
    - (nhóm 4)
14. **IME.** Đang soạn chữ thì không lọc, không typeahead, không điều hướng, không thêm tag (nhóm 2, 5, 6, 7).
15. **Chuỗi AT tiếng Anh viết cứng** ở Slider, NumberField, PinInput, Stepper, Pagination, Carousel, Calendar và các
    field ngày giờ. `primitives` phải bản địa hoá chúng (nhóm 6, 7, 8).

### 2.3 Con số hành vi

Đây là các giá trị cần giữ. Các nhịp của DOM không nằm ở đây, vì chúng là cơ chế: `setTimeout(0)`, `nextTick`, chờ
`click` khoảng 350 ms, cờ 1 ms. Chỉ thứ tự mà chúng tạo ra được giữ (§3).

| Hành vi | Giá trị | Nguồn |
| --- | --- | --- |
| Typeahead tự xoá | 1000 ms | `shared/useTypeahead.ts:16` |
| Cờ "đang đi qua vùng an toàn" của con trỏ | 300 ms | `shared/useGraceArea.ts:8` |
| Submenu mở, tính từ khi con trỏ bắt đầu di trên SubTrigger | 100 ms | `Menu/MenuSubTrigger.vue:73-78` |
| Vùng an toàn có hướng của submenu | 300 ms | `Menu/MenuSubTrigger.vue:28-29` |
| ContextMenu mở khi nhấn giữ | 700 ms | `ContextMenu/ContextMenuRoot.vue:41` |
| Tooltip: trễ mở; cửa sổ mở không trễ | 700 ms; 300 ms | `Tooltip/TooltipProvider.vue:72-73` |
| HoverCard: trễ mở; trễ đóng | 700 ms; 300 ms | `HoverCard/HoverCardRoot.vue:50-51` |
| NavigationMenu: trễ mở; trễ mở trong 300 ms sau khi đóng; trễ đóng; đang mở mà sang trigger khác | 200 ms; 150 ms; 150 ms; chuyển ngay | `NavigationMenu/NavigationMenuRoot.vue:108-109,142-149,194-207` |
| Select mở bằng nhấn chuột hoặc bút: nhả trong vòng này quanh chỗ nhấn thì không chọn; nhả xa hơn và ngoài nội dung thì đóng. Không áp cho chạm | 10 px | `Select/SelectContentImpl.vue:166-180` |
| Drawer: thả thì đóng | ≥ 40 px, hoặc > 0,3 px/ms | `Drawer/composables/useSwipeDismiss.ts:42,467-468` |
| Drawer: kéo ngược thì huỷ | > 10 px sau khi đã kéo > 20 px | `Drawer/composables/useSwipeDismiss.ts:42-43,398-407` |
| Drawer: vận tốc khi thả | 0 nếu mẫu cuối cũ hơn 80 ms | `Drawer/composables/useSwipeDismiss.ts:50,450-452` |
| Drawer có snap point: chỗ thả | từ 0,5 px/ms trở lên thì chiếu thêm 300 ms theo vận tốc, vận tốc tối đa 4 px/ms; tới snap point gần nhất; chỉ đóng khi gần trạng thái đóng hơn mọi snap point | `Drawer/composables/useDrawerSnapPoints.ts:7-9,136-160` |
| Toast: thời gian hiện | 5000 ms | `Toast/ToastProvider.vue:122` |
| Toast: vuốt bắt đầu; vuốt đóng | > 2 px (chuột), > 10 px (chạm); > 50 px | `Toast/ToastRootImpl.vue:289,301`; `Toast/ToastProvider.vue:124`; `Toast/utils.ts:42-50` |
| NumberField nhấn giữ | lặp sau 400 ms, mỗi 60 ms; chạm hoặc bút di quá 10 px thì huỷ | `NumberField/utils.ts:8-10,83-93` |
| ScrollArea: ẩn thanh cuộn; kiểu `scroll` coi là ngừng cuộn | 600 ms; 100 ms | `ScrollArea/ScrollAreaRoot.vue:55`; `ScrollArea/ScrollAreaScrollbarScroll.vue:55` |
| ScrollArea: thumb nhỏ nhất, bằng mức của macOS | 18 px | `ScrollArea/utils.ts:17-24` |
| Splitter: lề bắt handle | 15 px (con trỏ thô), 5 px (mịn) | `Splitter/SplitterResizeHandle.vue:136-140` |
| Splitter: bước phím | 10 %, Shift 100 % | `Splitter/utils/calculate.ts:47-55` |
| Pagination | 2 trang mỗi bên trang hiện tại; khi hiện trang đầu và cuối, số ô cố định là min(2s + 5, N) | `Pagination/PaginationRoot.vue:59`; `Pagination/utils.ts:27-76` |
| Mũi tên của popup | 10 × 5 px; khoảng cách tới anchor = side offset + chiều cao mũi tên | `shared/component/Arrow.vue:31-32`; `Popper/PopperContent.vue:293-296` |
| Lịch: dò ô tiếp theo dùng được | tối đa 24 bước | `shared/useCalendarKeyboardNavigation.ts:7-13` |
| Carousel | xoay mỗi 4000 ms, kéo bắt đầu sau 3 px, nuốt click sau 10 px, và các giá trị khác trong ghi chú chung của nhóm 8 | `Carousel/useCarousel.ts:42`; `Carousel/useCarouselViewport.ts:20-44` |

## 3. Hạ tầng: Rust và `gui` thay cơ chế DOM

Reka phải đoán từ DOM nhiều điều mà engine của ta biết chắc: lớp nào mở sau, node nào có focus, kích thước thật, khi
nào animation xong, event đến từ phím hay từ con trỏ. Mục này ghi, cho từng loại cơ chế DOM, bảo đảm mà nó phục vụ và
cách Rust hoặc `gui` đạt bảo đảm đó. Đây là hướng thiết kế cho S6, chưa phải API.

### 3.1 Engine biết chắc, reka phải đoán

| Cơ chế DOM của reka | Bảo đảm | Cách của `gui` và Rust |
| --- | --- | --- |
| Teleport, wrapper `position: fixed`, chép `z-index` | Nội dung nổi trên mọi thứ, không bị cha cắt | Lớp nổi là khái niệm của cây: `gui::deferred(Placement, …)` vẽ sau, có clip và thứ tự chồng riêng trong property tree (ADR-0016). Không di chuyển node |
| `data-dismissable-layer` và so vị trí trong `querySelectorAll` | Lớp mở sau là "bên trong" | Engine giữ stack lớp theo thứ tự mở, gắn với cây sở hữu (`Scope`). Hôm nay `gui` đã có danh sách lớp theo thứ tự vẽ (`Frame::layers`), và bấm ra ngoài đã coi lớp vẽ sau là bên trong. Định nghĩa "bên trong" là một quyết định (§4) |
| `pointer-events: none` trên `body` | Modal: bên ngoài trơ với con trỏ | Một luật trong hit test: dưới lớp modal trên cùng, không node nào trúng. Hôm nay dialog đạt điều này nhờ một lớp phủ kín cửa sổ; luật trong hit test vẫn đúng khi lớp modal không có lớp phủ |
| `overflow: hidden` trên `body`, bù thanh cuộn, chặn `touchmove` trên iOS | Nền không cuộn và không xô lệch | Engine định tuyến cuộn: dưới lớp modal không nhận wheel hay kéo. Thanh cuộn do ta vẽ nên layout không đổi, không có gì để bù |
| Gói `aria-hidden` (`hideOthers`) | AT chỉ thấy lớp modal | Cây AccessKit: nút của lớp có cờ modal, phần còn lại bị ẩn trong cùng một `TreeUpdate`, sau khi focus đã vào (ADR-0021) |
| FocusGuards (span canh ở mép `body`), `focusin`/`focusout`, `TreeWalker` tìm phần tử tab được, `MutationObserver` | Tab không thoát scope; biết khi node đang có focus bị gỡ | Bộ quản lý focus của `gui` tính thứ tự Tab từ cây giữ lại (`focusable`, `FocusHandle`) và giữ stack scope. Nó biết node bị gỡ khi `Scope` của node bị huỷ. Không cần lính canh, không cần observer. Hôm nay Tab đã vòng trong lớp trên cùng có node focus được |
| `setTimeout(0)`, `nextTick`, double rAF để sắp thứ tự | Cú nhấn mở lớp không đóng lớp đó; focus trả về sau khi cú nhấn xong; nền bị ẩn sau khi focus vào | Scheduler (ADR-0013, pre và post flush) và event có dấu thời gian (ADR-0015) cho thứ tự xác định. "Cùng một cú nhấn" là cùng pointer id và cùng chuỗi event, không phải một hẹn giờ |
| Chờ `click` cho thao tác chạm (trễ khoảng 350 ms của trình duyệt) | Tap xong mới đóng; cuộn thì không đóng | Tầng input của `gui` nhận dạng tap: nhấn và nhả trong ngưỡng xê dịch. Không có trễ của trình duyệt |
| Presence đoán `animationName` qua `getComputedStyle`; `animationend`; `animation-fill-mode` | Nội dung còn hiện tới hết animation đóng | `gui::motion` sở hữu animation, nên biết part có animation thoát hay không và khi nào nó xong. Nó đã giữ cây con đang rời được vẽ và trơ tới hết animation thoát (ADR-0007). Hiện diện là một enum do engine chuyển trạng thái, theo đồng hồ của ADR-0015 |
| floating-ui, `ResizeObserver` đo mũi tên, biến CSS `--reka-popper-*` | Vị trí, lật, đẩy vào trong viewport; look biết cạnh, gốc, kích thước khả dụng | `gui::deferred(Placement, …)` đã lật, đẩy vào trong và giới hạn kích thước như floating-ui, và căn theo item cho Select (`placement.rs`), từ kích thước thật. S3 của M10 trả kết quả có kiểu (`side`, `align`, `available`, `anchor`, `origin`; ADR-0022) để look đọc |
| Đo khi chặn animation, biến CSS kích thước nội dung | Accordion và Collapsible animate theo kích thước thật | Layout biết kích thước tự nhiên; animation đọc nó trực tiếp |
| `querySelectorAll('[data-reka-collection-item]')`, lọc `data-disabled` | Thứ tự hiển thị; bỏ item tắt | `gui::Members<T>` do layout điền theo thứ tự cây (đã có, ADR-0013). Item mang state tắt có kiểu. Danh sách ảo (ADR-0014: `Collection`, `View`, `ExtentTree`) cho thứ tự và điều hướng chạy trên dữ liệu, không trên các hàng đang mount |
| `tabindex` 0/−1, CustomEvent entry focus, cờ "focus do chuột" 1 ms, sửa lỗi Safari | Một tab stop; phân biệt focus bằng phím với focus bằng chuột | Bộ quản lý focus biết chế độ nhập từ event có dấu (`gui::focus_visible`). Entry focus là một hàm của state có kiểu |
| `refAutoReset`; chữ hiển thị làm nhãn dự phòng | Typeahead xoá sau 1000 ms; nhãn mặc định là chữ hiển thị | Item có nhãn chữ có kiểu; hẹn giờ chạy theo đồng hồ frame (`gui::clock`) |
| `useVModel`: `undefined` khác `false`, cờ `isUpdating` | Hai chủ cho một state mà không vọng lại | `Bind<T>` và `Model<T>` của `gui` (đã có): "không đưa" là `None`; chủ được quyết một lần khi dựng |
| `useId`; `useTriggerId` với `MutationObserver` | `aria-controls`, `aria-labelledby` chỉ trỏ tới thứ có thật | Quan hệ AccessKit (`labelled_by`, `controls`, `described_by`) trỏ bằng `NodeId` có kiểu; engine chỉ phát quan hệ tới node đang có. Không có id dạng chuỗi |
| Input ẩn (`VisuallyHiddenInput`, `BubbleSelect`), `closest('form')` | Form thu giá trị, kiểm tra required, nghe thay đổi | Không có form native. Một mô hình form có kiểu: field đăng ký tên và giá trị có kiểu; required và disabled là kiểu. Reset là một quyết định riêng (§4) |
| `Intl.Collator('en')`, `@internationalized/date`, `@internationalized/number`, `Intl.DateTimeFormat` | So khớp chữ Unicode; số học lịch; định dạng theo locale | ICU4X theo locale của người dùng (collator, lịch, số, ngày giờ). Tốt hơn reka: collator của reka cố định ở `'en'` |
| Render item vào `DocumentFragment` khi Select đóng | Trigger có nhãn; typeahead chạy cả khi đóng | Option là dữ liệu có kiểu, có sẵn mà không cần mount |
| `v-memo`, `elementsFromPoint`, `ResizeObserver` | Không render lại khi chỉ vị trí đổi; biết content đè lên trigger khác; biết kích thước | Binding theo phase (ADR-0013); hit test của `gui`; layout có sẵn kích thước |
| `inert` cùng `aria-hidden` (slide ngoài tầm nhìn, toast vượt giới hạn) | Không focus, không trúng con trỏ, AT không đọc | Một cờ trơ có kiểu trên cây con, áp cùng lúc cho focus, hit test và AccessKit |
| `contenteditable`, `beforeinput`, phát lại phím cho IME (TimeField) | Chữ chỉ đổi qua luật segment; IME nhập như gõ phím | Ô nhập và IME của `gui` (ADR-0009) có trạng thái soạn chữ tường minh; segment là node của ta |
| CSS scroll snap; `scrollend` với dự phòng 120 ms (Carousel) | Dừng đúng snap; biết lúc cuộn xong | Vật lý cuộn chạy trên render thread (ADR-0015), snap là dữ liệu; engine biết chính xác lúc dừng |
| Media query (con trỏ thô, giảm chuyển động); `localStorage` (Splitter) | Lề bắt rộng hơn cho cảm ứng; theo cài đặt hệ thống; layout bền qua phiên | Cài đặt hệ thống là signal từ tầng platform. Lưu trữ là một trait do app cung cấp, không có global ẩn |

### 3.2 Kiểu thay quy ước

- **Giá trị có kiểu.**
  - Primitive là generic theo `T: PartialEq`, hoặc `Eq + Hash` cho tập. Nó thay `AcceptableValue`, hàm `by` (tên khoá
    hoặc hàm) và phép so sâu của reka.
  - Mọi primitive dùng một luật so sánh: `PartialEq` của `T`. Reka hiện dùng bốn luật: `===` ở Switch và TagsInput, so
    cấu trúc ở Checkbox, RadioGroup và ToggleGroup, so theo khoá ở Tree (nhóm 5, 6).
- **Chọn một và chọn nhiều là hai kiểu.** Chọn một dùng `Option<T>`, chọn nhiều dùng một tập. Không có cờ `multiple`
  với giá trị lúc là `T` lúc là mảng. Không có `''` làm giá trị "xoá chọn": reka cấm item của Select và Combobox có giá
  trị `''` vì lý do đó (nhóm 5).
- **Máy trạng thái là enum, match đủ mọi nhánh.** Thiếu một nhánh thì compiler báo. Ví dụ:
  - hiện diện: mở, đang đóng, đã gỡ;
  - checkbox: bỏ chọn, chọn, chưa xác định;
  - avatar: đang tải, đã tải, lỗi;
  - ô phân trang: trang n, hoặc dấu ba chấm;
  - segment ngày giờ: rỗng, đang gõ, đủ.
- **Tổ hợp không hợp lệ thì không viết ra được.** Ví dụ:
  - AlertDialog không có lựa chọn "đóng khi bấm ra ngoài".
  - Ngày "không khả dụng" khác ngày "bị chặn" ở kiểu: ngày không khả dụng nhận focus, ngày bị chặn thì không (nhóm 7).
- **Một khái niệm "không dùng được", với hai mức.** Reka có ba khái niệm "tắt" không thống nhất (nhóm 2, 6, 8). Ta dùng
  hai mức:
  - **Tắt:** không focus, không gửi, điều hướng bỏ qua.
  - **Không khả dụng:** vẫn focus được và vẫn được đọc, nhưng không thao tác được. Ví dụ: checkbox chạm giới hạn của
    nhóm, ngày không khả dụng, nút Prev/Next của Carousel.
- **Sự kiện phủ quyết có lý do.** Việc đóng mang một lý do có kiểu: Esc, nhấn ngoài, focus ra ngoài, hoặc chọn item.
  Handler trả "cho phép" hoặc "huỷ". Cách này thay các CustomEvent đặt theo tên của reka.
- **Bảng phím là dữ liệu.** Mỗi primitive có một bảng `phím → hành động` cố định. RTL là một phép lật trên bảng. Test
  và devtools đọc chính bảng đó.
- **Con số hành vi là hằng có kiểu** (`Duration`, px), gom ở một chỗ, theo bảng §2.3.
- **Part và context.** Dùng một part ngoài root của nó là lỗi, và lỗi nêu tên root cần có (reka:
  `shared/createContext.ts:39-47`). Khi được, root trao cho part một handle có kiểu, để lỗi đó thành lỗi compile.
- **Chuỗi AT đi qua một catalog bản địa hoá có kiểu.** Không có chuỗi tiếng Anh viết cứng.
- **Không cấp phát mỗi frame.** Đổi state là ghi signal; danh sách item dùng lại `Members`.

### 3.3 Chia việc giữa `gui`, `primitives` và `shadcn`

- **`gui`, engine dùng chung:**
  - stack lớp và phát hiện bấm ra ngoài;
  - modal: hit test, định tuyến cuộn, cờ modal của AccessKit;
  - bộ quản lý focus: stack scope, bẫy focus, Tab vòng, trả focus;
  - chế độ nhập (phím hay con trỏ), nhận dạng tap và nhấn giữ;
  - `Placement`;
  - hiện diện theo animation;
  - quan hệ AccessKit có kiểu;
  - `Members`, hẹn giờ theo đồng hồ frame, cờ trơ.
- **`primitives`:**
  - máy trạng thái và bảng phím của từng widget: engine Menu, Dialog, Popover, Tooltip, Select, Listbox, Combobox, lõi
    lưới lịch, lõi segment, Slider, NumberField…;
  - các khối roving, typeahead và chọn trên `Members`;
  - mô hình form.
- **`shadcn`:** chỉ look. Nó đọc `Design`, `Theme` và state của primitive (các dòng "Look cần đọc").
- **Hôm nay** (sau C6, 2026-11-21):
  - `gui` đã có:
    - `FocusHandle`, `set_focus(…, by_keyboard)` và `focus_visible`;
    - danh sách lớp theo thứ tự vẽ (`Frame::layers`; mỗi lớp có root và anchor). Lớp dựng bằng `deferred(Placement, …)`,
      với lật, đẩy, giới hạn kích thước và căn theo item (`placement.rs`);
    - bấm ra ngoài theo lớp (`on_click_outside`): nhấn vào một lớp vẽ sau, hoặc vào anchor của lớp, không tính là ra
      ngoài. Đây là luật "bên trong" theo thứ tự của reka. Nhưng nó chỉ xét nút chính, trong khi bảo đảm 3 ở §2.2 xét
      mọi nút và xét chạm theo tap;
    - phím tới node có focus và tổ tiên của nó trước, rồi mới tới root của lớp trên cùng nếu focus ở lớp khác. Vì thứ
      tự đó, chưa có gì bảo đảm Esc chỉ tới lớp trên cùng;
    - Tab vòng trong lớp trên cùng có node focus được;
    - `Members`, `Bind`/`Model`/`use_model`, `Slot`, `provide`/`use_context`, `after_layout`, `clock::after`;
    - `motion`, giữ cây con đang rời được vẽ và trơ tới hết animation thoát.
  - `gui` chưa có:
    - đóng lớp do engine quản: lý do đóng, phủ quyết, focus ra ngoài;
    - modal mà engine biết: hôm nay dialog chặn con trỏ và cuộn nhờ lớp phủ kín cửa sổ, nhưng hit test, định tuyến
      cuộn và AccessKit không biết lớp nào là modal;
    - focus scope: tự focus vào nội dung khi mở, scope trên tạm dừng scope dưới, phân biệt bẫy với không bẫy;
    - nhận dạng tap và nhấn giữ;
    - cờ trơ công khai cho một cây con bất kỳ, như slide ngoài tầm nhìn hay toast vượt giới hạn. Cờ trơ hiện chỉ có
      trong `motion`, cho cây con đang rời;
    - quan hệ AccessKit: `labelled_by`, `controls`, `described_by`, active descendant.
  - Phần còn lại nằm trong `shadcn`:
    - trả focus dùng chung `dialog::return_focus` (6 file);
    - Esc viết riêng ở 7 file;
    - khi mở, dialog chỉ bỏ focus khỏi trang (`set_focus(None, …)`), không đưa focus vào nội dung như reka, nên AT
      không được dẫn vào dialog.

## 4. S6 cần quyết định

Với mỗi mục: chọn, rồi ghi lại. Một luật chung thì ghi vào ADR. Khác Chrome mà tốt hơn thì ghi vào
`verdict/better.tsv`.

1. **Phần engine của S6 nằm trong `gui`.** M10 chưa có task cho các phần sau: đóng lớp do engine quản, modal mà engine
   biết, focus scope, tap và nhấn giữ, cờ trơ, quan hệ AccessKit (§3.3). Đề xuất tách S6 làm hai: phần `gui` trước, rồi
   crate `primitives` dựng trên nó.
2. **"Bên trong" một lớp nghĩa là gì.**
   - Reka theo thứ tự DOM: mọi lớp mở sau đều là bên trong. Vì vậy một cú nhấn vào lớp mở sau không bao giờ đóng lớp
     mở trước.
   - FocusScope của reka lại theo cây component.
   - Hai định nghĩa khác nhau khi một lớp không liên quan mở sau (nhóm 1).
   - Ta có cả cây sở hữu lẫn thứ tự mở. Chọn một định nghĩa, và ghi chỗ khác reka. Hôm nay bấm ra ngoài của `gui` theo
     thứ tự lớp, như reka.
3. **Trả focus khi đóng.** Thống nhất cho mọi primitive: không giành lại focus mà thứ khác đã lấy. Drawer của reka không
   làm vậy khi đóng theo cách thường (nhóm 3).
4. **Esc khi IME đang soạn chữ.** Reka không xét trường hợp này (nhóm 3).
5. **Một luật so sánh giá trị và một khái niệm "không dùng được"** (§3.2).
6. **Bảng phím mặc định của từng primitive:** có vòng hay không, có PageUp/PageDown hay không. RovingFocus và
   `useArrowNavigation` của reka không thống nhất ở hai điểm này (nhóm 2).
7. **Mô hình form, và reset form.** Khi form reset, reka không reset các control tự dựng (nhóm 2).
8. **Luật gõ giờ.** Verdict so với Chrome, nên đây là một quyết định có chủ đích (nhóm 7):

   | | Gõ 2 rồi 5 | Backspace |
   | --- | --- | --- |
   | `gui::time_input`, theo Chrome | 23 | xoá cả segment |
   | reka | 5 | xoá một chữ số |
9. **Bản địa hoá:** chuỗi AT, AM/PM, chữ giữ chỗ tiếng Việt, chữ số không phải ASCII (nhóm 6, 7, 8).
10. **Thông báo cho AT** khi lịch đổi trang, hoặc khi một lần bấm chọn khoảng bị bỏ qua. Nhóm ngày của reka không có
    live region nào (nhóm 7).
11. **Tùy chọn chỉ đọc lúc dựng** (Accordion, Tabs): ta nên phản ứng khi chúng đổi (nhóm 8).
12. **Separator trong Toolbar** có xoay vuông góc với toolbar hay không (nhóm 4).
13. **Bug và khoảng trống của reka, không chép.** Làm theo ý định và ghi lại:
    - Nhóm 3: Drawer giành lại focus khi đóng. AlertDialog vẫn focus vào Cancel khi caller đã chặn auto-focus. Toast
      đóng bằng Esc thì không trả focus.
    - Nhóm 4: Menubar dùng cứng `ArrowRight` để mở submenu, cả khi RTL. NavigationMenu bỏ qua hướng đọc khi đi bằng
      mũi tên. NavigationMenu vừa bị click đóng mà chuột còn trên trigger thì mở lại sau 200 ms. Group không có Label
      vẫn trỏ `aria-labelledby`.
    - Nhóm 5: Listbox nhấn End khi chưa có highlight thì tới item đầu.
    - Nhóm 6: NumberField đổi giá trị bằng Home/End khi readonly. Stepper không kiểm tra bước disabled khi nhảy tới.
      PinInput dán cả vào ô disabled. Toggle không phải button vẫn đảo khi disabled. Thumb của Slider thiếu trạng thái
      disabled cho AT. Slider so khoảng cách giữa các thumb bằng số thực thô, nên step thập phân có thể bị từ chối sai;
      ta so theo số bước.
    - Nhóm 8: Avatar không hiện fallback khi delay bằng 0. Home/End của Accordion không bỏ qua trigger disabled.
      Splitter đảo Home/End khi RTL, và không cho AT biết hướng, tên hay trạng thái tắt. ScrollArea kiểu `scroll` không
      bao giờ vào trạng thái "đang tương tác".
14. **Tốt hơn Chrome, ghi vào `better.tsv`:**
    - Select chọn nhiều có `aria-multiselectable`.
    - Ô lọc của `command` có vai combobox và nối với danh sách của nó.
    - TagsInput: khi dán, cắt khoảng trắng và bỏ mảnh rỗng; nút xoá tag có tên hành động (nhóm 5).
    - Look `command` trong Chrome không làm mờ item tắt và không hiện dấu chọn, vì nó đọc một trạng thái mà reka không
      bao giờ đặt. Cần quyết định khớp Chrome hay sửa (nhóm 5).
    - Ví dụ lịch của shadcn-vue sai trong Chrome: Week Numbers và Multiple truyền tham số mà wrapper không đọc; Booked
      Dates làm cả lịch disabled (nhóm 7).
15. **Danh sách ảo.** Mọi bảo đảm của danh sách phải đúng trên toàn bộ dữ liệu, không chỉ phần đang mount (nhóm 2, 5).
    Reka vá việc này bằng typeahead và Home/End trên dữ liệu, và mất khả năng phân biệt các item trùng nhãn.
    `gui` đã có nền cho việc này: danh sách ảo theo chỉ số dữ liệu (ADR-0014), và hàng có focus cùng hàng active luôn
    được dựng (ADR-0021). S6 đặt roving, typeahead và chọn dải trên nền đó.

## 5. Nhóm 1: lớp nổi và focus

### DismissableLayer
`DismissableLayer/` · dùng bởi: Dialog (→ AlertDialog), Drawer, Popover, Menu (→ DropdownMenu, ContextMenu, Menubar), Select, Combobox (→ Autocomplete), HoverCard, Tooltip, NavigationMenu. Nhánh (`DismissableLayerBranch`): Toast viewport. Qua đó, shadcn-vue: `dialog`, `alert-dialog`, `sheet`, `command`, `drawer`, `popover`, `dropdown-menu`, `context-menu`, `menubar`, `select`, `combobox`, `hover-card`, `tooltip`, `sidebar`, `navigation-menu`. (shadcn-vue dùng `sonner`, không dùng Toast của reka.)

- **Vai trò:** Một lớp nổi đóng được. Giữ ngăn xếp lớp toàn cục. Quyết định Escape, nhấn con trỏ bên ngoài, focus ra ngoài có yêu cầu đóng lớp nào. Cho caller phủ quyết. Ở chế độ modal, làm mọi thứ bên dưới trơ với con trỏ.
- **Trạng thái:**
  - Không sở hữu open/closed. Chỉ phát một yêu cầu "đóng" cho chủ; chủ (Dialog, Popover…) đổi trạng thái mở (`DismissableLayer/DismissableLayer.vue:51-56`; `Dialog/DialogContentImpl.vue:113`).
  - Ngăn xếp lớp toàn cục, theo thứ tự lớp trở nên hiện diện. Lớp tự vào khi phần tử có mặt và đang hiện; tự ra khi ẩn hoặc bị gỡ. Không ai khác ghi (`DismissableLayer/context.ts:4-8`; `DismissableLayer/DismissableLayer.vue:197-222`).
  - Cờ "chặn con trỏ bên ngoài" (modal; reka `disableOutsidePointerEvents`). Caller ghi, mặc định tắt. Đổi khi đang mở có hiệu lực ngay và không đổi thứ tự stack (`DismissableLayer.vue:82`, `165-195`, `197-201`).
  - Tập nhánh toàn cục: caller đăng ký vùng không bao giờ là "bên ngoài", với mọi lớp (`DismissableLayer/DismissableLayerBranch.vue:19-29`).
  - Cờ hiện diện nội bộ, mặc định bật. Primitive chủ ghi cho nội dung giữ mount khi đóng (`DismissableLayer.vue:71-84`).
  - Bốn điểm phủ quyết: Escape; nhấn bên ngoài; focus bên ngoài; "tương tác bên ngoài" (hợp của hai cái sau, nhận cùng một sự kiện). Phủ quyết ở bất kỳ điểm nào → không đóng. Phủ quyết chỉ huỷ việc đóng; sự kiện gốc vẫn đi tiếp (`DismissableLayer.vue:119-158`).
- **Bảo đảm:**
  - Escape (`keydown`) chỉ do lớp trên cùng xử lý; lớp dưới không nhận. Mỗi lần nhấn đóng tối đa một lớp (`DismissableLayer.vue:146-158`; `DismissableLayer/DismissableLayer.test.ts:513-529`).
  - Escape đã được một điều khiển bên trong dùng (huỷ mặc định, hoặc chặn lan truyền trước khi tới cửa sổ) thì không đóng lớp. Ví dụ: ô lọc của menu xoá chữ trước, lần Escape sau mới đóng (`DismissableLayer.vue:157`; `DropdownMenu/DropdownMenuFilter.vue:110-115`).
  - Chuột và bút: đóng ngay lúc nhấn xuống (`pointerdown`) bên ngoài, như menu của hệ điều hành, với mọi nút chuột (`DismissableLayer/utils.ts:70-74`, `97-159`, `149-151`).
  - Cảm ứng: đóng ở `click`, sau khi nhấc tay. Chạm biến thành cuộn, kéo hay nhấn giữ thì không đóng. Tap đóng lớp modal không bấm xuyên vào phần tử bên dưới. Một tap ngoài đã bị huỷ không gây đóng ở tap sau bên trong (`DismissableLayer/utils.ts:110-117`, `130-148`, `153-157`; `DismissableLayer/DismissableLayer.test.ts:550-628`).
  - Cú nhấn làm mở lớp không đóng chính lớp đó (`DismissableLayer/utils.ts:160-176`).
  - "Bên trong" gồm: chính lớp, kể cả gốc của nó (FocusScope có thể focus gốc làm phương án cuối); mọi lớp đứng sau trong thứ tự tài liệu, tức lớp mở sau; mọi nhánh. Tương tác trong nhánh không phát sự kiện nào (`DismissableLayer/utils.ts:27-68`; `DismissableLayer.vue:120-125`, `134-139`).
  - Khi có lớp modal: chỉ lớp modal cao nhất và các lớp mở sau nó xét nhấn bên ngoài; lớp thấp hơn bỏ qua. Tính đủ điều kiện được chụp lúc sự kiện bắt đầu, nên lớp mới mở trong handler của cùng cú nhấn không đổi kết quả (`DismissableLayer.vue:106-112`; `DismissableLayer/utils.ts:92-107`).
  - Focus ra ngoài: chỉ khi focus *đến* một phần tử bên ngoài. Quyết định trễ 2 tick; nếu trong lúc đó focus đã bị kéo về trong lớp (trap) thì không tính. Focus rơi về body hoặc rời cửa sổ không tính, vì chỉ nghe `focusin` (`DismissableLayer/utils.ts:210-230`, `235-247`).
  - Modal: mọi thứ bên dưới không nhận hover, nhấn, focus bằng con trỏ. Cú nhấn bên ngoài chỉ đóng lớp, không kích hoạt gì bên dưới; người dùng nhấn lần nữa mới tới phần tử (`DismissableLayer.vue:19-24`). Lớp từ lớp modal cao nhất trở lên vẫn nhận con trỏ; lớp thấp hơn trơ như trang (`DismissableLayer.vue:102-112`, `231-237`).
  - Trạng thái trơ được đếm tham chiếu chung giữa các lớp modal và khoá cuộn. Chỉ khôi phục khi chủ cuối cùng nhả, theo bất kỳ thứ tự nào, về đúng giá trị trang có trước (`shared/bodyPointerEvents.ts:1-37`; `DismissableLayer/DismissableLayer.test.ts:83-486`).
  - Lớp ẩn nhưng giữ mount: rời stack; không xử lý Escape hay tương tác ngoài; không "để dành" tap đã làm nó hiện. Hiện lại → lên đỉnh stack (`DismissableLayer.vue:114-118`, `146-152`, `197-213`; `DismissableLayer/DismissableLayer.test.ts:681-753`).
  - Theo cửa sổ: Escape nghe trên cửa sổ của lớp; nhấn và focus ngoài xét trên tài liệu của lớp. Lớp trong iframe vẫn đúng (`DismissableLayer.vue:159-163`; `DismissableLayer/utils.ts:88-91`).
- **Thứ tự và lồng nhau:**
  - Đỉnh stack là lớp hiện diện gần nhất (`DismissableLayer.vue:94-100`, `153`).
  - Escape: chỉ đỉnh. Lớp kế tiếp nhận Escape khi đỉnh đã rời stack.
  - Nhấn bên ngoài: mỗi lớp đủ điều kiện tự xét. Nhấn ngoài mọi lớp → mọi lớp đủ điều kiện cùng đóng trong một cú nhấn (ví dụ popover trong dialog modal, nhấn overlay: cả hai đóng). Nhấn trong lớp con → cha không đóng. Nhấn trong cha, ngoài con → chỉ con đóng (suy từ `DismissableLayer/utils.ts:27-68` và `DismissableLayer.vue:106-131`).
  - Lồng nhau theo thứ tự DOM, không theo cây component: một lớp anh em không liên quan mở sau cũng là "bên trong" (`DismissableLayer/utils.ts:58-64`). Ngoại lệ: hai vùng Editable không coi nhau là lồng (`DismissableLayer/utils.ts:51-56`).
  - Focus ra ngoài không lọc theo modal: mọi lớp đều xét (`DismissableLayer/utils.ts:199-233`).
  - Chính sách caller dựng trên phủ quyết:
    - Modal Dialog, modal Popover, modal Menu, Select, Tooltip, HoverCard luôn chặn đóng do focus ra ngoài (`Dialog/DialogContentModal.vue:84-90`; `Popover/PopoverContentModal.vue:65`; `Menu/MenuRootContentModal.vue:34`; `Select/SelectContentImpl.vue:318`; `Tooltip/TooltipContentImpl.vue:114`; `HoverCard/HoverCardContentImpl.vue:92`).
    - Modal Dialog không đóng khi nhấn phải hoặc Ctrl + nhấn trái bên ngoài (`Dialog/DialogContentModal.vue:72-83`).
    - Lớp không modal bỏ qua tương tác trên trigger của chính nó; nếu không sẽ đóng rồi mở lại (`Dialog/DialogContentNonModal.vue:67-72`; `Popover/PopoverContentNonModal.vue:61-66`).
    - Escape trong submenu đóng cả cây menu (`Menu/MenuSubContent.vue:85-91`).
- **Look cần đọc:** không có. Trạng thái mở/đóng do primitive chủ cung cấp.
- **Cơ chế DOM không port:**
  - Thuộc tính `data-dismissable-layer` + so vị trí trong `querySelectorAll` → quy tắc "lớp mở sau là bên trong" (`DismissableLayer/utils.ts:41-64`).
  - `pointer-events: none` trên `body`, `auto`/`none` trên từng lớp → mọi thứ bên dưới trơ ở chế độ modal (`DismissableLayer.vue:231-237`; `shared/bodyPointerEvents.ts:15-37`).
  - Đăng ký `pointerdown` trên document trễ `setTimeout(0)` → cú nhấn đã mở lớp không đóng nó (`DismissableLayer/utils.ts:160-176`).
  - Chờ một `click` cho touch (trễ ~350 ms của trình duyệt) → đóng khi tap xong, không bấm xuyên (`DismissableLayer/utils.ts:130-148`).
  - `WeakMap` chụp tính đủ điều kiện ở pha capture → kết quả ổn định khi lớp mới mở giữa sự kiện (`DismissableLayer/utils.ts:83`, `92-96`).
  - CustomEvent không nổi bọt, huỷ được, phát trên target gốc → điểm phủ quyết (`shared/handleAndDispatchCustomEvent.ts:11-20`).
  - Cờ "đang ở trong cây" bật bằng handler capture trên lớp → nhận biết tương tác bên trong (`DismissableLayer/utils.ts:186-192`, `235-247`; `DismissableLayer.vue:238-240`).
  - Hai `nextTick` trước khi xét focus ngoài → focus bị trap kéo về không tính (`DismissableLayer/utils.ts:214-215`).
  - `await nextTick()` trước khi đọc phủ quyết nhấn ngoài → handler phủ quyết trễ trong cùng tick vẫn kịp (`DismissableLayer.vue:128`).
  - `ownerDocument`, `defaultView`, kiểm tra `Element` khác realm → lớp trong iframe (`DismissableLayer/utils.ts:17-25`; `DismissableLayer.vue:89-92`).
- **Dễ sót:**
  - Chuột đóng ở nhấn xuống, không ở click; cảm ứng thì ngược lại (`DismissableLayer/utils.ts:142-151`).
  - Mọi nút chuột đều tính là nhấn ngoài. Lọc nhấn phải là chính sách của caller (`DismissableLayer/utils.ts:97-159`; `Dialog/DialogContentModal.vue:72-83`).
  - Escape đã được điều khiển bên trong dùng thì không đóng (`DismissableLayer.vue:157`).
  - Phủ quyết "tương tác bên ngoài" chặn cả nhấn lẫn focus (`DismissableLayer.vue:126-130`, `140-143`).
  - Lớp đang chạy hiệu ứng thoát vẫn ở stack. Escape trong lúc đó bị lớp đang đóng nuốt; modal vẫn trơ tới khi thoát xong (`Dialog/DialogContent.vue:41-48`, `68`; `Dialog/DialogContentModal.vue:18-31`). Hành vi quan sát được, có thể không chủ ý. S6 nên quyết định.
  - Nhấn lên thanh cuộn của vùng cuộn bên ngoài nội dung (overlay cuộn) vẫn là nhấn ngoài và đóng dialog. Docs chỉ đưa cách vá (`docs:components/dialog.md:244-256`).

---

### Phát hiện tương tác bên ngoài
`DismissableLayer/utils.ts` (`usePointerDownOutside`, `useFocusOutside`, `isLayerExist`), `shared/onFocusOutside.ts` · dùng bởi: DismissableLayer; Editable dùng riêng, không vào stack lớp (`Editable/EditableRoot.vue:183-184`, `225-230`). Qua đó, shadcn-vue: như DismissableLayer (shadcn-vue không có `editable`).

- **Vai trò:** Báo khi người dùng nhấn, hoặc đưa focus, ra ngoài một vùng, theo đúng định nghĩa "bên trong" của hệ lớp.
- **Trạng thái:** Caller cấp vùng, hàm xử lý và cờ bật (`DismissableLayer/utils.ts:75-80`, `199-203`). Caller bật theo trạng thái của mình: lớp bật khi hiện diện, Editable bật khi đang sửa (`DismissableLayer.vue:131`; `Editable/EditableRoot.vue:183-184`). Người dùng ghi bằng nhấn và focus.
- **Bảo đảm:**
  - Định nghĩa "bên trong" như mục DismissableLayer (`DismissableLayer/utils.ts:27-68`).
  - Tắt → không nghe gì, và không giữ lại tap đã bắt đầu lúc tắt (`DismissableLayer/utils.ts:85-87`, `186-190`; `DismissableLayer.vue:114-118`).
  - Editable: nhấn hoặc focus ra ngoài khi đang sửa → gửi hoặc huỷ, theo chế độ gửi (`Editable/EditableRoot.vue:174-181`).
  - Nhấn vào một Editable khác không bị coi là "bên trong" chỉ vì nó đứng sau trong DOM; trừ khi nó nằm trong một lớp khác (`DismissableLayer/utils.ts:51-56`).
- **Thứ tự và lồng nhau:** không có stack riêng; dùng thứ tự DOM của mọi vùng đánh dấu lớp (`DismissableLayer/utils.ts:58-64`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** như DismissableLayer. `shared/onFocusOutside.ts` (dựa trên `focusout.relatedTarget`) được export nhưng không file nào trong core import (`shared/onFocusOutside.ts:5-29`; `shared/index.ts:15`).
- **Dễ sót:** nhấn ngoài có lọc "lớp dưới modal", còn focus ra ngoài thì không (`DismissableLayer/utils.ts:103-107` so với `199-249`).

---

### FocusScope
`FocusScope/` · dùng bởi: nội dung của Dialog (→ AlertDialog), Drawer, Popover, Menu (→ DropdownMenu, ContextMenu, Menubar), Select, Combobox. Tiện ích Tab dùng thêm ở NavigationMenu và Toast viewport (`NavigationMenu/NavigationMenuContentImpl.vue:20`; `Toast/ToastViewport.vue:27`). Qua đó, shadcn-vue: `dialog`, `alert-dialog`, `sheet`, `command`, `drawer`, `popover`, `dropdown-menu`, `context-menu`, `menubar`, `select`, `combobox`, `sidebar` (sheet trên mobile).

- **Vai trò:** Vùng focus. Khi mở thì đưa focus vào; khi đóng thì trả focus về; tuỳ chọn giữ focus (trap) và cho Tab vòng (loop). Các vùng xếp chồng; chỉ vùng trên cùng hoạt động.
- **Trạng thái:**
  - Caller ghi: giữ focus, mặc định tắt; Tab vòng, mặc định tắt; hiện diện, mặc định bật (cho nội dung giữ mount khi đóng) (`FocusScope/FocusScope.vue:22-46`, `69-79`). Ví dụ: Dialog luôn vòng, giữ focus khi modal và đang mở (`Dialog/DialogContentImpl.vue:93-97`; `Dialog/DialogContentModal.vue:61`). Select không vòng, không giữ, tự chặn Tab (`Select/SelectContentImpl.vue:302-304`, `204-206`).
  - Vùng sở hữu: phần tử focus cuối cùng bên trong; phần tử đang focus lúc mở (đích trả về); trạng thái tạm dừng; vị trí trong stack toàn cục; vùng cha trong cây vùng (`FocusScope.vue:83-102`, `259`; `FocusScope/stack.ts:10-38`).
  - Hai điểm phủ quyết:
    - Tự focus khi mở. Phủ quyết → không đưa focus vào; caller tự làm (Menu focus khung menu; Select focus mục đã chọn sau khi định vị) (`FocusScope.vue:221-237`; `Menu/MenuContentImpl.vue:253-263`; `Select/SelectContentImpl.vue:139-146`, `304`).
    - Tự focus khi đóng. Phủ quyết → không trả focus; caller tự chọn đích (`FocusScope.vue:273-291`; `Dialog/DialogContentModal.vue:63-71`).
  - Người dùng ghi focus bằng Tab, nhấn, hoặc code; trap sửa lại khi cần.
- **Bảo đảm:**
  - Mở: sau một tick, nếu focus chưa ở trong vùng → phát "tự focus khi mở". Không bị phủ quyết → focus điểm dừng Tab đầu tiên thật sự nhận được focus; nếu là input thì chọn hết chữ. Không có gì → focus chính vùng (`FocusScope.vue:225-237`, `250-271`; `FocusScope/utils.ts:80-87`).
  - Focus đã ở trong vùng lúc mở (ví dụ phần tử tự focus) → không phát, không đổi focus (`FocusScope.vue:260`, `270`).
  - Thứ tự Tab: tabindex dương tăng dần trước, rồi tabindex 0 theo thứ tự tài liệu. Bỏ phần tử disabled (kể cả trong `fieldset` disabled), `hidden`, input ẩn. Khi tìm biên, bỏ phần tử không hiển thị (`visibility: hidden`, `display: none` tính tới vùng). Một nhóm radio cùng tên là một điểm dừng: radio đã chọn; khi chưa chọn, radio nhìn thấy đầu tiên theo hướng Tab (`FocusScope/utils.ts:108-224`; `FocusScope/FocusScope.browser.test.ts:69-330`).
  - Tab (không Alt/Ctrl/Meta) khi giữ hoặc vòng: chỉ can thiệp khi không còn điểm dừng nhìn thấy theo hướng đang đi. Vòng → sang biên kia (Tab về đầu, Shift+Tab về cuối). Giữ mà không vòng → đứng yên ở biên. Không có điểm dừng nào và focus đang ở vùng → đứng yên (`FocusScope.vue:334-365`).
  - Giữ focus: focus đến bất kỳ phần tử ngoài vùng (Tab, nhấn, code) → kéo về phần tử focus cuối bên trong, chọn chữ nếu là input (`FocusScope.vue:119-150`). Focus về "không đâu" (cửa sổ mất focus) → để yên; trình duyệt nhớ focus khi cửa sổ quay lại (`FocusScope.vue:133-144`).
  - Giữ focus: phần tử đang focus bị gỡ khỏi vùng → focus về chính vùng, trừ khi focus đã ở chỗ khác trong vùng (`FocusScope.vue:152-189`).
  - Đóng: phát "tự focus khi đóng" ngay lúc gỡ. Sau một macrotask, nếu không bị phủ quyết và focus chưa "đi tiếp" → focus lại phần tử đã focus lúc mở (hoặc body), chọn chữ nếu là input (`FocusScope.vue:273-298`; `FocusScope/FocusScope.test.ts:439-473`).
  - "Focus đã đi tiếp" nghĩa là focus đang ở một phần tử không thuộc vùng, không thuộc vùng lồng trong nó theo cây component, và không phải body. Vùng cha đóng cùng lúc → dùng câu trả lời của cha, đo trước khi mọi handler chạy. Kết quả: lớp khác mở bằng chính cú nhấn đã đóng giữ được focus; đóng một chuỗi lớp lồng nhau trả focus về trigger ngoài cùng (`FocusScope/utils.ts:26-63`; `FocusScope.vue:279`, `289`; `DismissableLayer/DismissableLayer.browser.test.ts:67-121`, `204-226`).
  - Mọi focus do vùng gây ra không cuộn trang (`FocusScope/utils.ts:239-240`).
  - Hiện diện: ẩn → rời stack, không phát "tự focus khi đóng" (caller tự trả focus); hiện lại → lên đỉnh và tự focus như lần mount (`FocusScope.vue:302-332`; `Dialog/DialogContentModal.vue:45-53`).
  - Trong lúc trả focus, vùng mang dấu "đang tháo", để caller phân biệt blur do đóng lớp với blur của người dùng (ví dụ bỏ qua kiểm tra hợp lệ khi blur) (`FocusScope.vue:282-285`, `296`).
- **Thứ tự và lồng nhau:**
  - Một stack toàn cục. Vùng mount hoặc hiện sau lên đỉnh và tạm dừng vùng đỉnh cũ. Vùng tạm dừng không kéo focus, không xử lý Tab (`FocusScope/stack.ts:15-38`; `FocusScope.vue:120`, `129`, `197`, `337-338`).
  - Mọi vùng vào stack, kể cả vùng không giữ focus (Select, Combobox). Nhờ đó popup trong một modal nhận được focus dù nằm ngoài DOM của modal (`FocusScope.vue:72-79`; `FocusScope/FocusScope.test.ts:251-354`).
  - Vùng rời stack sau khi đã trả focus (cùng macrotask đó); vùng đỉnh mới được tiếp tục (`FocusScope.vue:286-298`; `FocusScope/stack.ts:34-37`).
  - Vùng mở và đóng trong cùng tick không để lại dấu: không kẹt trong stack, vùng kia vẫn giữ focus (`FocusScope.vue:242-252`; `FocusScope/FocusScope.test.ts:475-506`).
  - Quan hệ cha–con theo cây component, không theo DOM, vì vùng con thường bị portal (`FocusScope/utils.ts:9-24`; `FocusScope.vue:86-92`).
  - Vùng được thêm lại khi đang tạm dừng thì tiếp tục: đỉnh luôn hoạt động (`FocusScope/stack.ts:29-31`; `FocusScope/FocusScope.test.ts:508-540`).
  - Caller nhả giữ focus ngay khi yêu cầu đóng, trước khi hiệu ứng thoát xong (`Dialog/DialogContentModal.vue:61`; `Popover/PopoverContentModal.vue:44`; `Menu/MenuRootContentModal.vue:30`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:**
  - `tabindex="-1"` trên vùng → vùng nhận focus làm phương án cuối mà không thành điểm dừng Tab (`FocusScope.vue:371`).
  - `focusin`/`focusout` trên document hoặc shadow root, `composedPath`, `containsComposed` → giữ focus xuyên shadow DOM (`FocusScope.vue:111-118`, `191-207`; `FocusScope/utils.ts:65-74`).
  - `MutationObserver` → biết phần tử đang focus bị gỡ (`FocusScope.vue:160-189`, `208-210`).
  - Bỏ qua `relatedTarget === null` → tránh vòng lặp 100 % CPU của Chrome khi phần tử focus bị gỡ (`FocusScope.vue:133-144`).
  - `TreeWalker`, `tabIndex` lúc chạy, `getComputedStyle`, `:disabled` → suy ra điểm dừng Tab (`FocusScope/utils.ts:108-224`).
  - CustomEvent không nổi bọt, huỷ được (`focusScope.autoFocusOnMount`/`OnUnmount`) → hai điểm phủ quyết (`FocusScope/utils.ts:3-5`).
  - `setTimeout(0)` trước khi trả focus → để trình duyệt xong cú nhấn đã đóng lớp (`FocusScope.vue:286`).
  - `getActiveElement` đi xuyên shadow root (`shared/getActiveElement.ts:1-12`).
  - `focus({ preventScroll: true })` và `select()` (`FocusScope/utils.ts:232-250`).
- **Dễ sót:**
  - Chọn hết chữ input mỗi khi vùng đưa focus vào: lúc mở, lúc trap kéo về, lúc trả về (`FocusScope/utils.ts:242-248`; `FocusScope.vue:125`, `149`, `233`, `290`).
  - Trap chỉ kéo về phần tử focus cuối *bên trong*. Nếu chưa có (tự focus bị phủ quyết, chưa focus gì bên trong), focus thoát được (`FocusScope.vue:119-126`; `FocusScope/utils.ts:236-237`).
  - Đích trả về là phần tử đang focus *một tick sau mount*, không phải trigger. Dialog tự chọn: phần tử focus trước khi mở, rồi trigger, bỏ cái đã rời tài liệu (`FocusScope.vue:250-259`; `Dialog/utils.ts:88-94`). Docs nói Escape "moves focus to DialogTrigger" (`docs:components/dialog.md:369-370`), chưa đủ.
  - Link được tự focus như mọi điểm dừng. Hàm lọc link có nhưng không dùng (`FocusScope/stack.ts:50-52`; `FocusScope.vue:233`).
  - "Tự focus khi đóng" vẫn phát khi focus đã đi tiếp; chỉ hành động mặc định bị bỏ (`FocusScope/FocusScope.test.ts:449-459`).
  - Vùng không giữ focus vẫn tạm dừng trap bên dưới nó (`FocusScope.vue:72-79`).

---

### FocusGuards
`FocusGuards/`, `shared/useFocusGuards.ts` · dùng bởi: Popover (khi hiện), Menu (khi hiện; → DropdownMenu, ContextMenu, Menubar), Select, Combobox (`Popover/PopoverContentImpl.vue:52`; `Menu/MenuContentImpl.vue:133`; `Select/SelectContentImpl.vue:118`; `Combobox/ComboboxContentImpl.vue:64`). Dialog không dùng; chỉ story của DismissableLayer dùng (`DismissableLayer/story/_DummyDialog.vue:25`). Qua đó, shadcn-vue: `popover`, `dropdown-menu`, `context-menu`, `menubar`, `select`, `combobox`.

- **Vai trò:** Khi một lớp mở, đặt hai phần tử nhận focus vô hình ở đầu và cuối toàn tài liệu, để mọi lần focus đi/đến đều phát `focusin`/`focusout` mà trap bắt được (`shared/useFocusGuards.ts:8-14`).
- **Trạng thái:** Bộ đếm toàn cục số người cần guard. Caller bật/tắt, thường theo hiện diện. Người dùng không ghi gì (`shared/useFocusGuards.ts:5-6`, `15-39`).
- **Bảo đảm (trong trình duyệt):**
  - Tab ra khỏi phần tử cuối trang không rơi vào giao diện trình duyệt. Focus dừng ở guard; trap của FocusScope thấy và kéo về (`shared/useFocusGuards.ts:8-11`; `FocusScope/FocusScope.vue:119-126`).
  - Một cặp duy nhất cho mọi lớp; gỡ khi người cuối thôi dùng (`shared/useFocusGuards.ts:19-37`; `FocusGuards/FocusGuards.test.ts:37-56`).
  - Guard là điểm dừng Tab thật. Vì vậy nội dung giữ mount khi đóng phải tắt guard (`shared/useFocusGuards.ts:12-14`).
- **Thứ tự và lồng nhau:** dùng chung một cặp, đếm tham chiếu; thứ tự lớp không ảnh hưởng (`shared/useFocusGuards.ts:28-37`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** cả khối. `span[tabindex=0]` trong suốt, `position: fixed`, không nhận con trỏ, chèn ở `afterbegin`/`beforeend` của `body` → luôn có điểm dừng Tab ở mép tài liệu, để `focusin` phát và trap kéo focus về (`shared/useFocusGuards.ts:19-27`, `41-50`).
- **Dễ sót:**
  - Bảo đảm thật cần giữ chỉ có một: Tab trong lớp đang giữ focus không bao giờ đưa focus ra khỏi lớp hay cửa sổ. FocusScope đã đảm nhận việc này; engine không có "giao diện trình duyệt" để Tab thoát ra (`FocusScope/FocusScope.vue:334-365`).
  - Với lớp không giữ focus, guard tạo một điểm dừng Tab vô hình ở mép trang. Đây là phụ phẩm, không port (`shared/useFocusGuards.ts:12-14`).

---

### Presence
`Presence/`, `shared/useStateMachine.ts`, `shared/vShowWhenKept.ts` · dùng bởi: nội dung và overlay của Dialog (→ AlertDialog) và Drawer; nội dung của Popover, Menu và submenu, Select, Combobox, HoverCard, Tooltip, NavigationMenu (content, indicator, viewport), Toast; Collapsible content (→ Accordion), Tabs content, indicator của Checkbox, RadioGroup, MenuItem, SelectItem, ListboxItem; scrollbar của ScrollArea; FieldError. Qua đó, shadcn-vue: `dialog`, `alert-dialog`, `sheet`, `command`, `drawer`, `popover`, `dropdown-menu`, `context-menu`, `menubar`, `select`, `combobox`, `hover-card`, `tooltip`, `navigation-menu`, `accordion`, `collapsible`, `tabs`, `checkbox`, `radio-group`, `scroll-area`.

- **Vai trò:** Giữ một part còn sống sau khi bị yêu cầu ẩn, tới khi hiệu ứng thoát của nó xong. Không có hiệu ứng thoát thì gỡ ngay.
- **Trạng thái:**
  - Caller ghi: "được yêu cầu hiện" (bắt buộc); "luôn render", mặc định tắt. Khi bật, caller tự ẩn/hiện; Presence chỉ báo trạng thái (`Presence/Presence.ts:16-31`, `80-82`).
  - Presence sở hữu máy ba trạng thái (`Presence/usePresence.ts:28-40`):
    - `mounted` → `unmountSuspended` khi bị ẩn và hiệu ứng thoát bắt đầu; → `unmounted` khi bị ẩn mà không có hiệu ứng.
    - `unmountSuspended` → `unmounted` khi hiệu ứng thoát xong hoặc bị huỷ, hoặc phần tử mất; → `mounted` khi được hiện lại.
    - `unmounted` → `mounted` khi được hiện.
  - Xuất "đang hiện diện" = `mounted` hoặc `unmountSuspended`, cho con và cho caller (`Presence/usePresence.ts:193-195`; `Presence/Presence.ts:52-55`).
  - Bốn mốc caller quan sát được, không phủ quyết được: bắt đầu vào, xong vào, bắt đầu ra, xong ra (`Presence/usePresence.ts:42-56`).
- **Bảo đảm:**
  - Bị ẩn khi không có hiệu ứng thoát, hoặc khi part đang không hiển thị → gỡ ngay (`Presence/usePresence.ts:75-84`).
  - Bị ẩn khi hiệu ứng thoát bắt đầu → part còn sống (vẫn vẽ, vẫn ở mọi stack) tới khi chính hiệu ứng đó kết thúc hoặc bị huỷ (`Presence/usePresence.ts:85-96`, `112-124`).
  - Được hiện lại khi đang thoát → giữ nguyên instance: không remount, không mất state (`Presence/usePresence.ts:33-36`, `68-73`).
  - Bị ẩn giữa hiệu ứng vào → chờ hiệu ứng thoát. Tín hiệu huỷ của hiệu ứng vào không kết thúc việc chờ (`Presence/usePresence.ts:107-124`).
  - Chỉ hiệu ứng của chính part được tính; hiệu ứng của phần tử con bị bỏ qua (`Presence/usePresence.ts:112-114`, `143-148`; `Presence/Presence.test.ts:199-236`).
  - Part bị gỡ sớm (ví dụ cha bị gỡ) → coi như đã xong (`Presence/usePresence.ts:172-176`).
  - Không nháy: khung cuối của hiệu ứng thoát được giữ tới khi part bị gỡ (`Presence/usePresence.ts:125-137`).
  - Với nội dung định vị, hiệu ứng được đo trên chính nội dung, không trên khung định vị bọc ngoài (`Presence/Presence.ts:92-96`).
  - Con cháu của part đang thoát vẫn là hiện diện: lớp vẫn ở stack, khoá cuộn và ẩn nền vẫn giữ, focus chỉ trả về khi part bị gỡ (`Dialog/DialogContent.vue:41-48`, `68`; `Dialog/DialogContentModal.vue:18-31`; `FocusScope/FocusScope.vue:273-298`).
  - Nội dung giữ mount khi đóng (caller chọn) bị ẩn sau khi thoát và giữ state. Hiệu ứng vào chỉ chạy một lần khi gắn, không hai lần (`shared/vShowWhenKept.ts:13-30`; `Dialog/DialogContent.vue:50-54`, `67`).
- **Thứ tự và lồng nhau:** mỗi part có Presence riêng. Overlay và nội dung Dialog thoát độc lập. Dialog giữ khoá cuộn tới khi nội dung thoát xong, dù overlay đã gỡ (`Dialog/DialogOverlay.vue:34-50`; `Dialog/DialogContentModal.vue:18-31`).
- **Look cần đọc:** trạng thái mở hay đóng, để chọn hiệu ứng vào hay ra (vega: mờ dần + phóng 95 %, 100 ms — `shadcn-style:style-vega.css:443-449`). Ngược lại, primitive cần biết từ look: part có hiệu ứng thoát không, và khi nào nó xong.
- **Cơ chế DOM không port:**
  - Đọc `getComputedStyle().animationName` sau `nextTick` và so với tên cũ → đoán "hiệu ứng thoát đã bắt đầu", vì DOM không có sự kiện `animationrun` (`Presence/usePresence.ts:58-100`, `181-184`, `202-204`). Engine biết trực tiếp look có hiệu ứng thoát hay không.
  - `animationstart`/`animationend`/`animationcancel` + `CSS.escape` → biết hiệu ứng nào kết thúc (`Presence/usePresence.ts:112-157`).
  - `animation-fill-mode: forwards` + `setTimeout` → không nháy khung cuối (`Presence/usePresence.ts:125-137`).
  - CustomEvent `enter`/`after-enter`/`leave`/`after-leave` trên phần tử → bốn mốc (`Presence/usePresence.ts:42-49`).
  - Bắt buộc đúng một con, `renderSlotFragments`, callback `ref` → gắn vào phần tử thật (`Presence/Presence.ts:55-103`; `shared/renderSlotFragments.ts:4-13`).
  - `v-show` có điều kiện → ẩn nội dung giữ mount mà không chạy lại hiệu ứng vào (`shared/vShowWhenKept.ts:13-63`).
- **Dễ sót:**
  - Chỉ chờ CSS animation, không chờ CSS transition (`Presence/usePresence.ts:155-157`). Transition phải đi qua "luôn render" + `<Transition>` của Vue (`docs:guides/animation.md:50-52`). Mô tả "with transition support" (`docs:utilities/presence.md:3`) dễ hiểu sai.
  - Phát hiện "đang thoát" dựa vào đổi tên hiệu ứng: vào và ra dùng cùng một keyframes thì không chờ (`Presence/usePresence.ts:92-100`).
  - Docs sai về mốc. "Bắt đầu vào" phát khi part xuất hiện, không khi hiệu ứng bắt đầu. "Xong vào" phát ngay nếu không có hiệu ứng vào. "Bắt đầu ra" không phát khi tên hiệu ứng không đổi (`Presence/usePresence.ts:52-56`, `68-73`, `97-99` so với `docs:utilities/presence.md:40-61`).

---

### Teleport
`Teleport/` · dùng bởi: Portal của Dialog, AlertDialog, Drawer, Popover, Menu (→ DropdownMenu, ContextMenu, Menubar), Select, Combobox, HoverCard, Tooltip, Toast. Qua đó, shadcn-vue: mọi content trên đều bọc trong portal (ví dụ `shadcn:popover/PopoverContent.vue:31`, `shadcn:dialog/DialogContent.vue:31`).

- **Vai trò:** Vẽ nội dung lớp ở tầng trên cùng, không bị tổ tiên của trigger cắt hay cuộn, trong khi về logic vẫn thuộc cây component của trigger.
- **Trạng thái:** Caller ghi: đích (mặc định là đích toàn cục của ConfigProvider, rồi `body`); tắt (vẽ tại chỗ); hoãn tới khi đích tồn tại; ép render trước khi mount ở client (`Teleport/Teleport.vue:2-27`, `39`). Người dùng không ghi gì.
- **Bảo đảm:**
  - Nội dung lớp không chịu overflow, cắt, transform hay stacking của tổ tiên trigger.
  - Thứ tự vẽ giữa các lớp cùng tầng = thứ tự mount: mỗi portal nối vào cuối đích lúc mount (`vue:runtime-core/src/components/Teleport.ts:569-589`). shadcn đặt cùng `z-50` cho popover, tooltip, dialog, nên lớp mở sau vẽ trên lớp mở trước (`shadcn:popover/PopoverContent.vue:37`; `shadcn:tooltip/TooltipContent.vue:27`; `shadcn:dialog/DialogContent.vue:36`).
  - Context của root đi xuyên portal: part trong nội dung đọc được state của root (`Dialog/DialogContentImpl.vue:53`).
  - Chỉ render sau khi mount ở client, trừ khi bị ép (`Teleport/Teleport.vue:41-46`).
- **Thứ tự và lồng nhau:** lớp con portal ra khỏi DOM của lớp cha nhưng đứng sau nó. Điều này khớp quy tắc "mở sau là bên trong" của DismissableLayer (`DismissableLayer/utils.ts:58-64`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** Vue `<Teleport>` di chuyển DOM → thoát cắt và stacking. `useMounted` → tránh lệch hydrate SSR. `defer` → đích render sau. `to` và đích toàn cục của ConfigProvider → shadow DOM, custom element (`Teleport/Teleport.vue:30-53`; `ConfigProvider/ConfigProvider.vue:42-48`).
- **Dễ sót:**
  - Nội dung giữ mount khi đóng không được chuyển xuống cuối khi mở lại. Thứ tự vẽ và quy tắc "bên trong" theo lần mount đầu, còn stack Escape theo lần hiện gần nhất (`vue:runtime-core/src/components/Teleport.ts:569-589`; `DismissableLayer/DismissableLayer.vue:197-213`). S6 nên cho cả ba theo một thứ tự.
  - Portal đẩy nội dung xuống cuối thứ tự Tab của tài liệu. Reka bù bằng vòng Tab trong lớp (`Popover/PopoverContentImpl.vue:56-58`; `Dialog/DialogContentImpl.vue:93-96`) hoặc chặn Tab (`Menu/MenuContentImpl.vue:298-299`; `Select/SelectContentImpl.vue:204-206`). S6 phải tự định nghĩa Tab ở biên của lớp không giữ focus.
  - Chú thích của "ép render" nói về animation, nhưng code chỉ bỏ bước chờ mount ở client (`Teleport/Teleport.vue:22-26` so với `41-46`; lặp lại ở `meta:DialogPortal.md`).
  - Docs Dialog nói portal vào `body` (`docs:components/dialog.md:93`); source còn đọc đích toàn cục của ConfigProvider trước (`Teleport/Teleport.vue:39`).

---

### Popper
`Popper/`, `shared/component/Arrow.vue`, `shared/useSize.ts`, `shared/useDirection.ts` · dùng bởi: Popover; Menu và submenu (→ DropdownMenu, ContextMenu với anchor ảo tại con trỏ, Menubar); Select (chế độ popper); Combobox (→ Autocomplete); HoverCard; Tooltip; Toast positioner. Qua đó, shadcn-vue: `popover`, `dropdown-menu`, `context-menu`, `menubar`, `select`, `combobox`, `hover-card`, `tooltip`, `sidebar`.

- **Vai trò:** Đặt nội dung nổi cạnh một anchor: chọn cạnh và căn lề, tránh va chạm với biên, bám theo anchor, đặt mũi tên, và xuất hình học cho look.
- **Trạng thái:**
  - Anchor: part anchor tự đăng ký với root cùng họ. Hoặc caller cấp một hình chữ nhật hay một điểm ảo thay thế (`Popper/PopperAnchor.vue:30-32`; `Popper/PopperContent.vue:191-197`, `351-354`; `ContextMenu/ContextMenuTrigger.vue:39-56`).
  - Root theo họ (popover, menu, tooltip, select, combobox, hover-card). Anchor tìm root gần nhất *cùng họ*, xuyên qua root của họ khác. Nhờ vậy trigger lồng nhau (một nút vừa là trigger Popover vừa là trigger Tooltip) đăng ký đúng root (`Popper/PopperRootContext.ts:5`, `38-53`; `Popper/PopperRootContext.test.ts:26-75`; `Popper/PopperComposition.browser.test.ts:104-160`).
  - Caller ghi cấu hình, mặc định (`Popper/PopperContent.vue:15-33`, `35-202`):
    - cạnh ưu tiên: dưới; khoảng cách tới anchor: 0 px;
    - căn: giữa; lệch căn (tính từ start/end): 0 px;
    - tránh va chạm: bật; lật sang cạnh đối diện: bật; lật căn: bật; thử cạnh vuông góc: không (hoặc start, end);
    - biên va chạm: viewport, cộng phần tử thêm; đệm va chạm: 0, một số hoặc từng cạnh;
    - đệm từ mũi tên tới góc: 0; ẩn mũi tên khi không chỉ được vào tâm anchor: bật;
    - dính trên trục căn: `partial` (ở trong biên khi anchor còn một phần trong biên) hoặc `always`;
    - ẩn khi anchor bị che hết: tắt; ưu tiên vị trí (có thể đè anchor): tắt;
    - đo lại mỗi frame: tắt (`optimized`); cập nhật khi layout xê dịch: bật;
    - hướng chữ: theo ConfigProvider, mặc định `ltr` (`Popper/PopperContent.vue:249`; `shared/useDirection.ts:11-16`).
  - Xuất, chỉ đọc: cạnh và căn thực tế sau va chạm; vị trí; đã định vị chưa, kèm mốc "đã đặt"; vị trí mũi tên và cờ mũi tên bị ẩn; gốc biến hình; rộng/cao khả dụng; rộng/cao của anchor; cờ anchor bị che (`Popper/PopperContent.vue:316-349`, `373-397`).
  - Người dùng ghi gián tiếp: cuộn, đổi kích thước, di chuyển anchor → định vị lại (`Popper/PopperContent.vue:362-367`).
- **Bảo đảm:**
  - Vị trí mong muốn = cạnh + căn; căn giữa không có hậu tố (`Popper/PopperContent.vue:256-260`).
  - Khoảng cách tới anchor = khoảng cách khai báo + chiều cao mũi tên, nếu có mũi tên (`Popper/PopperContent.vue:293-296`).
  - Mặc định: lệch → trượt dọc trục căn để ở trong biên (`partial` hoặc `always`) → lật nếu cạnh ưu tiên va chạm → đo khả dụng → mũi tên → gốc biến hình → ẩn khi bị che (`Popper/PopperContent.vue:291-349`, `146-154`).
  - Lật mặc định chỉ thử cạnh đối diện. Hai cạnh của trục đều chật → vẫn ở trên trục đó.
  - Khi bật thử cạnh vuông góc:
    - cạnh trái/phải: start thử trên rồi dưới, end thử dưới rồi trên;
    - cạnh trên/dưới: start thử trái rồi phải, end ngược lại, đảo trong RTL (`Popper/PopperContent.vue:66-78`; `Popper/Popper.browser.test.ts:71-102`).
  - Ưu tiên vị trí: lật trước, rồi trượt trên cả hai trục. Nội dung luôn trong biên và có thể đè anchor (`Popper/PopperContent.vue:182-189`, `297-309`).
  - Tắt tránh va chạm: không lật, không trượt, không thử cạnh vuông góc. Tắt lật cạnh: không thử cạnh vuông góc (`Popper/PopperContent.vue:283-315`; `Popper/Popper.browser.test.ts:104-119`).
  - Trước khi định vị xong: nội dung không hiện ở chỗ sai và không chạy hiệu ứng vào. Hiệu ứng vào bắt đầu khi đã biết cạnh cuối cùng (`Popper/PopperContent.vue:415`, `449-453`, `467-471`).
  - Chiều rộng nội dung giữ kích thước tự nhiên (`max-content`), không bị biên bóp. Look tự giới hạn bằng rộng khả dụng (`Popper/PopperContent.vue:416`; `shadcn:popover/PopoverContent.vue:37`).
  - Mũi tên nằm trên mép nội dung đối diện cạnh đã đặt, chỉ về anchor, cách góc ít nhất bằng đệm mũi tên. Bị ẩn (vẫn chiếm chỗ) khi không chỉ được vào tâm anchor. Mặc định 10 × 5 px (`Popper/PopperArrow.vue:6-11`, `35-62`; `Popper/PopperContent.vue:339-340`, `385-388`; `shared/component/Arrow.vue:30-34`).
  - Gốc biến hình: mũi tên hiện → ở đầu mũi tên. Không có hoặc bị ẩn → ở mép nội dung hướng về anchor, tại điểm căn: start 0 %, center 50 %, end 100 %. Trên cạnh trên/dưới, trục ngang đảo trong RTL; cạnh trái/phải không đảo (`Popper/utils.ts:14-68`; `Popper/Popper.test.ts:28-131`).
  - Anchor bị che hết (khi bật): nội dung vô hình và không nhận con trỏ, nhưng vẫn mở (`Popper/PopperContent.vue:346-347`, `423-429`).
  - Bám anchor khi cuộn, đổi kích thước, layout xê dịch. Chế độ `always` đo lại mỗi frame (`Popper/PopperContent.vue:362-367`).
  - Nội dung giữ mount mà đang ẩn: ngừng bám, giữ vị trí cuối, để hiệu ứng thoát kết thúc tại chỗ (`Popper/PopperContent.vue:351-354`). ContextMenu giữ anchor ảo trong lúc thoát (`ContextMenu/ContextMenuTrigger.vue:58-59`).
  - RTL áp cho cả định vị lẫn gốc biến hình (`Popper/PopperContent.vue:249`, `412`; `Popper/Popper.test.ts:133-170`).
- **Thứ tự và lồng nhau:** mỗi nội dung định vị độc lập. Anchor theo họ gần nhất (trên). Submenu đặt ở cạnh phải (trái nếu RTL), căn start (`Menu/MenuSubContent.vue:65-66`).
- **Look cần đọc:** cạnh và căn thực tế (hướng trượt vào: `shadcn-style:style-vega.css:883`); gốc biến hình (phóng từ gốc); rộng/cao khả dụng (giới hạn kích thước: popover, select, dropdown-menu); rộng/cao của anchor (rộng tối thiểu bằng trigger: select, combobox, dropdown-menu; `shadcn:select/SelectContent.vue:39`, `50`); vị trí mũi tên và cờ ẩn; "đã định vị" (để giữ hiệu ứng vào).
- **Cơ chế DOM không port:**
  - floating-ui (`useFloating`, `offset`, `flip`, `shift`, `limitShift`, `size`, `arrow`, `hide`, `autoUpdate`), ngoài references/ (`Popper/PopperContent.vue:208-218`, `356-371`) → toàn bộ việc tính vị trí.
  - Wrapper `position: fixed` + `transform`, `minWidth: max-content` (`Popper/PopperContent.vue:409-431`) → toạ độ theo viewport, không bị bóp.
  - `translate(0, -200%)` trước khi định vị (`Popper/PopperContent.vue:415`) → không hiện sai chỗ khi đo.
  - `animation: none` tới khi định vị (`Popper/PopperContent.vue:449-453`) → hiệu ứng vào dùng cạnh cuối.
  - Biến CSS `--reka-popper-*` và `data-side`/`data-align`, được từng họ đổi tên (`Popper/PopperContent.vue:318-337`, `418-421`, `447-448`; `Popover/PopoverContentImpl.vue:82-91`) → kênh xuất hình học cho look.
  - Sao chép `z-index` của nội dung lên wrapper (`Popper/PopperContent.vue:390-394`, `417`) → thứ tự chồng look khai báo vẫn đúng.
  - `ResizeObserver` đo mũi tên (`shared/useSize.ts:12-58`) → khoảng cách và gốc theo kích thước thật của mũi tên.
  - `v-memo` (`Popper/PopperContent.vue:432-456`) → không render lại cây nội dung khi chỉ vị trí đổi.
  - `dir` trên wrapper (`Popper/PopperContent.vue:412`) → floating-ui đọc RTL.
  - `altBoundary` khi có biên tuỳ chỉnh (`Popper/PopperContent.vue:274-281`) → floating-ui tôn trọng biên với `fixed`.
- **Dễ sót:**
  - Chiều cao mũi tên cộng vào khoảng cách (`Popper/PopperContent.vue:293-296`).
  - Mặc định trượt trước, lật sau (`Popper/PopperContent.vue:297-315`).
  - Không thử cạnh vuông góc theo mặc định. Hai cạnh của trục đều chật thì nội dung ở lại trên trục đó, không nhảy sang cạnh vuông góc (`Popper/PopperContent.vue:72`; `Popper/Popper.browser.test.ts:73-81`).
  - Mũi tên ẩn khi nội dung trượt xa; gốc biến hình lúc đó về điểm căn (`Popper/utils.ts:25-28`).
  - Chế độ mặc định không đo mỗi frame: anchor di chuyển bằng hiệu ứng hay transform không được bám (`Popper/PopperContent.vue:362-367`). Chú thích "on every animation frame" mô tả sai chế độ mặc định (`Popper/PopperContent.vue:168-173`).
  - Chú thích (và docs) nói lật căn "chỉ xảy ra khi ưu tiên vị trí", nhưng code truyền cờ lật căn cho cả hai nhánh (`Popper/PopperContent.vue:95-101` so với `283-315`). Có thể đúng trong thực tế vì trượt chạy trước; chưa kiểm được trong floating-ui.
  - Bảng props trong docs không ghi mặc định (`meta:PopoverContent.md`); mặc định thật ở `Popper/PopperContent.vue:15-33`.

---

### Khoá cuộn và trơ con trỏ
`shared/useBodyScrollLock.ts`, `shared/bodyPointerEvents.ts` · dùng bởi: Dialog modal (overlay giữ, nội dung giữ thêm), Drawer (overlay), Popover modal, Menu modal (→ DropdownMenu, ContextMenu; Menubar luôn không modal), Select (mặc định bật), Combobox (tuỳ chọn); DismissableLayer modal dùng chung bộ đếm trơ con trỏ. Export public (`index.ts:59`). Nguồn: `Dialog/DialogOverlayImpl.vue:19-25`; `Dialog/DialogContentModal.vue:21-31`; `Drawer/DrawerOverlayImpl.vue:20`; `Popover/PopoverContentModal.vue:16-19`; `Menu/MenuContentImpl.vue:136`; `Menubar/MenubarMenu.vue:59`; `Select/SelectContentImpl.vue:111`, `119`; `Combobox/ComboboxContentImpl.vue:62-63`. Qua đó, shadcn-vue: `dialog`, `alert-dialog`, `sheet`, `command`, `drawer`, `popover` (khi modal), `dropdown-menu`, `context-menu`, `select`, `combobox`.

- **Vai trò:** Khi có ít nhất một người giữ khoá: trang không cuộn, không nhận con trỏ, và không xê dịch layout vì thanh cuộn biến mất. Người cuối nhả → mọi thứ về đúng như cũ.
- **Trạng thái:**
  - Bản đồ toàn cục: mỗi người dùng một khoá bật/tắt; trạng thái chung = có khoá nào bật (`shared/useBodyScrollLock.ts:14-26`, `142-158`). Caller ghi khoá của mình (`shared/useBodyScrollLock.ts:148-151`).
  - Cấu hình ứng dụng: cách bù thanh cuộn. Mặc định cộng độ rộng thanh cuộn vào padding phải; hoặc dùng margin; hoặc một giá trị cố định; hoặc không bù (`shared/useBodyScrollLock.ts:64-86`; `ConfigProvider/ConfigProvider.vue:29-36`, `66`).
  - Bộ đếm trơ con trỏ dùng chung với lớp modal, theo từng tài liệu (`shared/bodyPointerEvents.ts:1-37`).
- **Bảo đảm:**
  - Khoá khi có ít nhất một người giữ; mở khi người cuối nhả, bất kể thứ tự. Bật/tắt nhiều lần trong một tick không làm kẹt khoá (`shared/useBodyScrollLock.ts:20-26`, `49-57`, `131-133`; `shared/useBodyScrollLock.test.ts:57-115`).
  - Không xê dịch layout: bù đúng độ rộng thanh cuộn vừa ẩn, cộng vào padding đang có (kể cả padding từ stylesheet). Trả lại đúng giá trị inline cũ (`shared/useBodyScrollLock.ts:34-47`, `62-89`; `shared/useBodyScrollLock.test.ts:133-300`).
  - Trang trơ với con trỏ khi khoá, chung bộ đếm với lớp modal (`shared/useBodyScrollLock.ts:41`, `131-136`).
  - Giữ `overflow` mà trang đã đặt (`shared/useBodyScrollLock.ts:43`, `59-60`).
  - Cảm ứng (iOS): chặn kéo một ngón làm cuộn trang; vẫn cho cuộn trong vùng cuộn được; cho đa chạm (pinch zoom). Dịch chuyển ≤ 0.01 px không tính là kéo, để không nuốt tap (`shared/useBodyScrollLock.ts:91-129`, `161-197`; `shared/useBodyScrollLock.ios.test.ts:37-160`).
- **Thứ tự và lồng nhau:** một khoá chung cho mọi lớp. Dialog: overlay giữ khoá; nội dung giữ thêm một phần tới khi thoát xong (`Dialog/DialogContentModal.vue:18-31`; `Dialog/DialogOverlayImpl.vue:19-25`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** `overflow: hidden` trên `body` → chặn cuộn tài liệu. Padding/margin phải + biến `--scrollbar-width` trên `html` → bù thanh cuộn đã ẩn. `pointer-events: none` trên `body`, đếm tham chiếu → trơ con trỏ. Listener `touchmove` không passive trên iOS → chặn cuộn cảm ứng. `createSharedComposable` → bản đồ khoá chung (`shared/useBodyScrollLock.ts:34-137`).
- **Dễ sót:**
  - Khoá cuộn cũng làm trang trơ con trỏ, không chỉ chặn cuộn (`shared/useBodyScrollLock.ts:131-136`).
  - Modal Dialog không có overlay thì không khoá cuộn; trang vẫn cuộn được (`Dialog/DialogContentModal.vue:18-20`, `28-30`).
  - Ngưỡng nhiễu 0.01 px (`shared/useBodyScrollLock.ts:112-116`).
  - Reka chỉ khoá cuộn của tài liệu. Vùng cuộn bên trong trang không nhận cuộn vì trang trơ con trỏ, còn bàn phím bị trap giữ trong lớp (`shared/useBodyScrollLock.ts:131-136`; `FocusScope/FocusScope.vue:119-150`). Giá trị S6 cần giữ: khi modal mở, không gì phía sau cuộn hay nhận con trỏ.

---

### Ẩn phần còn lại khỏi AT
`shared/useHideOthers.ts` · dùng bởi: Dialog modal, Popover modal, Menu modal (→ DropdownMenu, ContextMenu), Select, Combobox, Drawer (`Dialog/DialogContentModal.vue:37-38`; `Popover/PopoverContentModal.vue:24`; `Menu/MenuRootContentModal.vue:23`; `Select/SelectContentImpl.vue:126`; `Combobox/ComboboxContentImpl.vue:65-66`; `Drawer/DrawerContent.vue:50`). Qua đó, shadcn-vue: `dialog`, `alert-dialog`, `sheet`, `command`, `drawer`, `popover` (khi modal), `dropdown-menu`, `context-menu`, `select`, `combobox`.

- **Vai trò:** Khi lớp modal mở, AT chỉ thấy lớp đó; phần còn lại bị ẩn khỏi cây truy cập. Đóng thì trả lại.
- **Trạng thái:** Đích (phần tử lớp, hoặc không có) do primitive ghi, theo hiện diện (`shared/useHideOthers.ts:11`, `20`). Người dùng không ghi.
- **Bảo đảm:**
  - Phần còn lại bị ẩn (`aria-hidden="true"`) chỉ sau khi focus đã vào lớp. Focus không bao giờ nằm trong phần bị ẩn (`shared/useHideOthers.ts:26-31`, `42`; `shared/useHideOthersFocus.browser.test.ts:91-129`).
  - Phần còn lại được trả trước khi focus về trigger, cả khi đích bị bỏ lẫn khi lớp bị gỡ (`shared/useHideOthers.ts:14-18`, `45-46`; `shared/useHideOthersFocus.browser.test.ts:123-128`).
  - Modal lồng: modal trong ẩn luôn nội dung của modal ngoài. Đóng modal trong → nội dung modal ngoài hiện lại trước khi focus về trigger nằm trong nó; trang vẫn ẩn (`shared/useHideOthersFocus.browser.test.ts:132-151`).
  - Lớp không modal không ẩn gì (`shared/useHideOthersFocus.browser.test.ts:109-112`).
  - Đổi đích trực tiếp → trả cái cũ rồi ẩn theo cái mới. Một chủ đóng không gỡ phần ẩn của chủ khác còn mở. Đóng trước khi kịp ẩn → không ẩn (`shared/useHideOthers.ts:14-33`; `shared/useHideOthers.browser.test.ts:43-134`).
- **Thứ tự và lồng nhau:** ẩn và trả theo stack modal. Với focus: ẩn sau khi focus vào; trả trước khi focus ra (`shared/useHideOthers.ts:6-10`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** gói `aria-hidden` (`hideOthers`) đặt `aria-hidden` lên các phần tử ngoài đích; thuật toán ngoài references/ (`shared/useHideOthers.ts:3`, `42`) → AT chỉ thấy lớp modal. Hai `nextTick` → chờ FocusScope tự focus (`shared/useHideOthers.ts:28-31`). Bỏ qua phần tử nằm trong `[popover]` native đang đóng (`shared/useHideOthers.ts:34-41`). Tắt khi chạy test (`shared/useHideOthers.ts:23`).
- **Dễ sót:**
  - Thứ tự so với focus ở cả hai chiều (trên).
  - Combobox lấy phần tử gốc của combobox làm đích, không lấy danh sách (`Combobox/ComboboxContentImpl.vue:65-66`).

---

### Vùng an toàn của con trỏ
`shared/useGraceArea.ts` · dùng bởi: Tooltip (nội dung hover được), HoverCard (`Tooltip/TooltipContentHoverable.vue:16-21`; `HoverCard/HoverCardContentImpl.vue:25-31`). Menu có bản riêng cho submenu (`Menu/MenuSubTrigger.vue:82-125`; `Menu/MenuContentImpl.vue:243-251`). Qua đó, shadcn-vue: `tooltip`, `sidebar`, `hover-card`; bản Menu: submenu của `dropdown-menu`, `context-menu`, `menubar`.

- **Vai trò:** Khi con trỏ rời trigger đi tới nội dung (hoặc ngược lại), cho một vùng an toàn để lớp không đóng khi con trỏ băng qua khoảng trống.
- **Trạng thái:** Sở hữu đa giác an toàn (hoặc không có) và cờ "đang đi qua", tự tắt sau 300 ms (`shared/useGraceArea.ts:8`, `16`). Phát tín hiệu "con trỏ đã thoát" (`shared/useGraceArea.ts:17`, `71`). Chỉ chuyển động con trỏ của người dùng ghi. Caller đọc cờ để không mở tooltip khác và không đóng hover card khi con trỏ đang đi qua (`Tooltip/TooltipTrigger.vue:75-84`; `HoverCard/HoverCardTrigger.vue:25-33`); caller đóng khi có tín hiệu thoát (`Tooltip/TooltipContentHoverable.vue:19-21`; `HoverCard/HoverCardContentImpl.vue:29-31`).
- **Bảo đảm:**
  - Rời trigger → vùng là bao lồi của hai điểm cách điểm rời 1 px (lùi vào trong phần tử vừa rời, theo mép gần nhất) và bốn góc nội dung. Rời nội dung → tương tự, hướng về trigger (`shared/useGraceArea.ts:24-52`, `90-149`, `173-234`).
  - Khi có vùng, mỗi lần con trỏ di chuyển: vào trigger hoặc nội dung → bỏ vùng, không đóng; ra khỏi vùng → bỏ vùng và đóng; đè lên một trigger hover khác → bỏ vùng và đóng (`shared/useGraceArea.ts:54-79`).
  - Con trỏ đứng yên trong vùng → không đóng; chỉ di chuyển mới được xét (`shared/useGraceArea.ts:54-79`).
  - Cờ "đang đi qua" bật khi tạo vùng; tắt sau 300 ms (đứng yên, cuộn), khi bỏ vùng, hoặc khi gỡ (`shared/useGraceArea.ts:7-22`; `shared/useGraceArea.test.ts:159-175`).
  - Bản Menu (submenu): submenu mở sau 100 ms hover trên trigger (`Menu/MenuSubTrigger.vue:66-80`).
  - Rời trigger khi submenu đã mở → vùng 5 điểm: điểm rời lùi 5 px ra xa submenu (vào trong trigger) để luôn nằm trong vùng, cộng bốn góc của submenu (`Menu/MenuSubTrigger.vue:93-109`).
  - Vùng chỉ có tác dụng khi con trỏ đang đi về phía cạnh của submenu. Nó hết hạn sau 300 ms, tuỳ chỉnh được (`Menu/MenuSubTrigger.vue:5-13`, `29`, `111-115`; `Menu/MenuContentImpl.vue:243-251`, `325-341`; `Menu/utils.ts:90-99`).
  - Trong vùng, hover mục khác không đổi highlight và không đóng submenu (`Menu/MenuSubTrigger.vue:8`; `Menu/MenuContentImpl.vue:353-377`).
  - Chỉ áp cho chuột (`Menu/MenuSubTrigger.vue:67-68`, `83-84`).
- **Thứ tự và lồng nhau:** mỗi cặp trigger–nội dung có vùng riêng. Một trigger hover khác bị đè lên thì đóng ngay (`shared/useGraceArea.ts:64`, `69-72`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** `pointerleave`/`pointermove` trên document và `getBoundingClientRect` → hình học vùng. `refAutoReset` → hẹn giờ 300 ms. Thuộc tính `[data-grace-area-trigger]` → nhận ra trigger hover khác (`shared/useGraceArea.ts:64`; `Tooltip/TooltipTrigger.vue:127`; `HoverCard/HoverCardTrigger.vue:65`).
- **Dễ sót:**
  - Đệm 1 px, không phải mặc định 5 px của hàm (`shared/useGraceArea.ts:30`, `110`).
  - 300 ms chỉ tắt cờ, không xoá vùng (`shared/useGraceArea.ts:8`, `54-79`).
  - Hai thuật toán khác nhau: bao lồi đối xứng, không hạn giờ (tooltip, hover card); vùng có hướng, có hạn giờ (menu).

---

### Chế độ nhập: bàn phím hay con trỏ
`shared/useIsUsingKeyboard.ts` · dùng bởi: Menu (→ DropdownMenu, ContextMenu, Menubar), Carousel. Qua đó, shadcn-vue: `dropdown-menu`, `context-menu`, `menubar` (carousel của shadcn-vue dùng embla, không dùng reka).

- **Vai trò:** Một cờ toàn cục "người dùng đang dùng bàn phím", để quyết định focus khi mở lớp.
- **Trạng thái:** Cờ bật khi có `keydown`, tắt khi có `pointerdown` hoặc `pointermove`. Chỉ người dùng ghi (`shared/useIsUsingKeyboard.ts:4-20`).
- **Bảo đảm (qua Menu):**
  - Mở menu bằng con trỏ → focus vào khung menu, không highlight mục đầu. Bằng bàn phím → focus mục đầu (`Menu/MenuContentImpl.vue:420-424`, `253-263`).
  - Mở submenu bằng bàn phím → focus vào submenu. Bằng con trỏ → focus ở lại (`Menu/MenuSubContent.vue:70-73`).
- **Thứ tự và lồng nhau:** một cờ chung cho cả ứng dụng (`shared/useIsUsingKeyboard.ts:22`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** listener capture, passive trên window; `createSharedComposable` (`shared/useIsUsingKeyboard.ts:7-22`) → cờ đúng trước mọi handler.
- **Dễ sót:** cờ cập nhật trước mọi handler của cùng sự kiện; chỉ di chuyển con trỏ cũng đủ tắt cờ (`shared/useIsUsingKeyboard.ts:8-16`).

---

### Liên kết ID trigger ↔ nội dung
`shared/useTriggerId.ts` (`useTriggerId`, `useContentId`, `useContentMounted`, `useRenderedContentId`) · dùng bởi: trigger và nội dung của Dialog, Drawer, Popover, DropdownMenu, Menubar, submenu, Select, Combobox (→ Autocomplete), Tooltip, NavigationMenu. Accordion, Collapsible, Tabs chỉ dùng phần giải id, không dùng phần chờ nội dung render (`Accordion/AccordionTrigger.vue:19`; `Collapsible/CollapsibleContent.vue:39`; `Tabs/utils.ts:27`). Qua đó, shadcn-vue: `dialog`, `alert-dialog`, `sheet`, `drawer`, `popover`, `dropdown-menu`, `menubar`, `context-menu` (submenu), `select`, `combobox`, `tooltip`, `navigation-menu`.

- **Vai trò:** Trigger chỉ tham chiếu tới nội dung (`aria-controls`, hoặc `aria-describedby` với tooltip) khi nội dung thật sự đang được render, bằng đúng id mà nội dung mang.
- **Trạng thái:** Id của trigger và của nội dung nằm trên context của root. Ưu tiên: id caller đặt > id đã có > id sinh ra. Sau khi render, id phần tử thật mang luôn thắng (`shared/useTriggerId.ts:34-91`). Một bộ đếm số nội dung đang render, theo root (`shared/useTriggerId.ts:108-206`). Không ai khác ghi.
- **Bảo đảm:**
  - Tham chiếu chỉ có khi đang mở và nội dung đã có phần tử; không bao giờ trỏ tới id không tồn tại (`shared/useTriggerId.ts:208-218`; `Dialog/DialogTrigger.vue:40-42`; `Popover/PopoverTrigger.vue:40-42`; `Select/SelectTrigger.vue:216-217`; `Menu/MenuSubTrigger.vue:157-159`; `Tooltip/TooltipTrigger.vue:120-122`).
  - Nội dung giữ mount mà đang đóng không được tham chiếu (điều kiện "đang mở" ở các trigger trên).
  - Tham chiếu xuất hiện và biến mất trong cùng lượt cập nhật thêm/bỏ nội dung, không chậm một lượt; một lượt thứ hai sẽ xáo thứ tự việc đưa focus vào (`shared/useTriggerId.ts:117-123`).
  - Id caller đặt trên trigger hoặc nội dung luôn được tôn trọng, kể cả khi đổi sau mount (`shared/useTriggerId.ts:34-70`).
- **Thứ tự và lồng nhau:** bộ đếm theo root; submenu đếm trên context của menu (`Menu/MenuSubTrigger.vue:54`).
- **Look cần đọc:** không có.
- **Cơ chế DOM không port:** `MutationObserver` cho nội dung `asChild` có slot rỗng; xử lý `KeepAlive`; ưu tiên id từ HTML server khi hydrate (`shared/useTriggerId.ts:72-77`, `135-205`) → id và tham chiếu khớp với cái thật sự có trong tài liệu.
- **Dễ sót:** server và lần render hydrate đầu không có tham chiếu (`shared/useTriggerId.ts:208-213`). Chỉ liên quan SSR.

---

### Ghi chú chung cho nhóm 1
- **Hiện diện, không phải tồn tại.** Mọi khối ở đây xoay quanh "đang hiện", không phải "đang mount": stack lớp, stack focus, guard, khoá cuộn, ẩn nền, bám vị trí, tham chiếu ARIA. Nội dung giữ mount khi đóng rời mọi stack, ngừng mọi hiệu ứng phụ, rồi lên đỉnh khi hiện lại (`DismissableLayer/DismissableLayer.vue:197-213`; `FocusScope/FocusScope.vue:302-332`; `shared/useFocusGuards.ts:12-14`; `Menu/MenuContentImpl.vue:133-175`; `Popper/PopperContent.vue:351-354`). Trong lúc thoát, part vẫn là hiện diện (Presence). Menu giữ mount mở lại như mới: không mục nào highlight, không còn chuỗi gõ tìm (`Menu/MenuContentImpl.vue:167-171`).
- **Hai định nghĩa lồng nhau.** DismissableLayer theo thứ tự DOM: mở sau là bên trong (`DismissableLayer/utils.ts:58-64`). FocusScope theo cây component (`FocusScope/utils.ts:9-13`). Hai cái khác nhau khi một lớp không liên quan mở sau. S6 có một cây sở hữu giữ lại; cần chọn một định nghĩa và ghi chỗ khác reka.
- **Hai stack khác nhau.** Stack lớp có Tooltip, HoverCard (không có vùng focus). Stack focus có mọi vùng, kể cả vùng không giữ focus. Cả hai: đỉnh là cái hiện diện gần nhất.
- **Thứ tự khi mở modal:** focus vào → ẩn nền khỏi AT. Khi đóng: trả nền → trả focus (`shared/useHideOthers.ts:6-10`). Ở reka đây là một chuỗi tick. Trong một lần cập nhật cây truy cập, hai việc có thể đi cùng nhau, miễn nút đang focus không bao giờ nằm trong phần bị ẩn.
- **Giá trị thời gian cần giữ:** 300 ms cờ đi qua vùng an toàn (`shared/useGraceArea.ts:8`); submenu mở sau 100 ms, vùng có hướng 300 ms (`Menu/MenuSubTrigger.vue:29`, `75-78`); cảm ứng đóng ở `click`; nhiễu cảm ứng 0.01 px (`shared/useBodyScrollLock.ts:114`); mũi tên 10 × 5 px (`shared/component/Arrow.vue:30-34`). Còn lại (`setTimeout(0)`, `nextTick` một hoặc hai lần, chờ `click` ~350 ms) là nhịp của DOM, không phải giá trị hành vi. Chỉ giữ thứ tự chúng tạo ra: lớp không bị đóng bởi cú nhấn đã mở nó; focus trả về sau khi cú nhấn kết thúc; nền bị ẩn sau khi focus vào.
- **Phủ quyết** chỉ huỷ hành động mặc định; sự kiện vẫn tới mọi người nghe (`DismissableLayer/DismissableLayer.browser.test.ts:89-94`; `FocusScope/FocusScope.test.ts:449-459`).
- **Docs khác source (source thắng):** mốc của Presence (`docs:utilities/presence.md:40-61` so với `Presence/usePresence.ts:52-99`); "with transition support" (`docs:utilities/presence.md:3`), trong khi chỉ CSS animation được chờ; chú thích "ép render" của Teleport (`Teleport/Teleport.vue:22-26`); chú thích chế độ cập nhật và lật căn của Popper (`Popper/PopperContent.vue:95-101`, `168-173`); cảnh báo "trap cần ít nhất một phần tử focus được" (`docs:utilities/focus-scope.md:74-78`), trong khi source có phương án cuối là focus chính vùng (`FocusScope/FocusScope.vue:232-236`, `351-353`, `371`); đích portal (`docs:components/dialog.md:93` so với `Teleport/Teleport.vue:39`); đích trả focus của Dialog (`docs:components/dialog.md:369-370` so với `Dialog/utils.ts:88-94`).
- **Ngoài references/:** floating-ui, gói `aria-hidden`, vueuse. Hành vi của chúng ở đây chỉ mô tả qua cấu hình reka truyền vào và qua test của reka.
- **Không thuộc nhóm này:** RovingFocus, `useArrowNavigation`, `useTypeahead`, VisuallyHidden, Primitive/Slot. Primitive dùng các khối trên (Dialog, Popover, Menu, Select…) do nhóm khác mô tả.

## 6. Nhóm 2: tập hợp, điều hướng, controlled state và form

### RovingFocus
`RovingFocus/` · dùng bởi: `Tabs`, `RadioGroup`, `ToggleGroup`, `Toolbar`, `Menubar`, `Menu` (nội dung của `DropdownMenu`, `ContextMenu`, `Menubar`), `CheckboxGroup`, `TagGroup`, `Tree` → shadcn-vue `tabs`, `radio-group`, `toggle-group`, `menubar`, `dropdown-menu`, `context-menu`.

- **Vai trò:** roving tabindex. Cả nhóm là một Tab stop. Mũi tên chuyển focus giữa các item. Nhóm nhớ item nào là Tab stop.
- **Trạng thái:**
  - Tab stop hiện tại: id của item duy nhất nhận Tab. User ghi khi focus hoặc nhấn chuột vào một item focusable (`RovingFocusGroup.vue:134-136`; `RovingFocusItem.vue:132-141`). Caller: controlled → giá trị là của caller, nhóm chỉ xin đổi; uncontrolled → giá trị đầu, đọc một lần (`RovingFocusGroup.vue:75-78`). Chưa có → mọi item ra khỏi thứ tự Tab, chính container nhận Tab (`RovingFocusGroup.vue:152`; `RovingFocusItem.vue:126`).
  - Trạng thái tạm, chỉ user ghi: "đang Shift+Tab ra" (`RovingFocusGroup.vue:79,137-139,161`); "focus đến từ chuột" (`:80,115,118-123,158`). Cấu trúc ghi: số item focusable (`:81,140-145`; `RovingFocusItem.vue:41-59`).
  - Caller cấu hình nhóm: trục (ngang, dọc, hoặc không đặt = cả hai), hướng (riêng > `ConfigProvider` > `ltr`), loop (mặc định tắt), cuộn tới item khi entry focus (mặc định có) (`RovingFocusGroup.vue:66-74`).
  - Caller cấu hình item: focusable (mặc định có); "active" = item đại diện cho giá trị (tab đang chọn, radio đang check, toggle đang bấm); id riêng (không có → id sinh) (`RovingFocusItem.vue:4-15,27-34`; `Tabs/TabsTrigger.vue:40-41`; `RadioGroup/RadioGroupItem.vue:106-107`; `ToggleGroup/ToggleGroupItem.vue:39`).
  - Báo ra caller: entry focus (hủy được); Tab stop đổi (`RovingFocusGroup.vue:39-42,98-100`).
- **Bảo đảm:**
  - Bảng phím (`RovingFocus/utils.ts:9-18`): `ArrowLeft`, `ArrowUp` → trước; `ArrowRight`, `ArrowDown` → sau; `Home`, `PageUp` → đầu; `End`, `PageDown` → cuối.
  - Trục dọc bỏ qua Left/Right. Trục ngang bỏ qua Up/Down. Không đặt trục → cả bốn mũi tên (`utils.ts:38-42`).
  - `rtl` đổi Left↔Right trước khi tra bảng. Up/Down/Home/End không đổi (`utils.ts:20-28,37`).
  - Loop tắt: dừng ở đầu/cuối. Loop bật: vòng (`RovingFocusItem.vue:88-98`; test `RovingFocus/RovingFocus.test.ts:76-132`).
  - "Sau/trước/đầu/cuối" = item đầu tiên thực sự nhận được focus theo thứ tự ứng viên. Ứng viên từ chối focus bị bỏ qua (`utils.ts:45-55`).
  - Item không focusable: bị bỏ qua khi đi mũi tên và khi entry focus (`RovingFocusItem.vue:83,129`; `RovingFocusGroup.vue:103`); nhấn chuột vào nó không lấy focus (`RovingFocusItem.vue:136`).
  - Tab vào nhóm (container nhận focus bằng bàn phím): focus nhảy vào item theo ưu tiên active → highlighted (vd. item menu dưới con trỏ, `Menu/MenuItemImpl.vue:36,86`) → Tab stop hiện tại → item bật đầu tiên (`RovingFocusGroup.vue:103-112`).
  - Focus vào container bằng chuột không gây entry focus (`RovingFocusGroup.vue:90-97,158`).
  - Caller hủy entry focus → focus ở lại container. `Menu` hủy khi menu mở bằng chuột (`Menu/MenuContentImpl.vue:420-424`).
  - Tab từ một item ra khỏi nhóm, không sang item kế (test `RovingFocus/RovingFocus.test.ts:43-46`).
  - Shift+Tab từ item ra khỏi nhóm trong một lần nhấn. Container tạm rời thứ tự Tab, để focus không rơi vào container rồi bị kéo lại vào item (`RovingFocusItem.vue:65-68`; `RovingFocusGroup.vue:152`).
  - Nhóm không còn item focusable → không phải Tab stop (`RovingFocusGroup.vue:152`).
  - Mũi tên kèm Ctrl/Alt/Meta không di chuyển. Kèm Shift cũng không, trừ item cho phép (Tree cho phép để chọn dải) (`RovingFocusItem.vue:80-81`; `Tree/TreeItem.vue:181`).
  - Phím đã được phần tử trong cùng xử lý (đã chặn) → bỏ qua (`RovingFocusItem.vue:62-63`). Phím phát từ con cháu của item → bỏ qua (`:70-71`). Riêng Shift+Tab từ con cháu vẫn được ghi nhận (`:65-68` đứng trước `:70`).
  - Phím điều hướng được xử lý thì bị tiêu thụ (trang không cuộn) (`RovingFocusItem.vue:82`).
  - Item nhận focus được cuộn vào tầm nhìn. Chỉ entry focus có thể tắt việc này (`RovingFocusGroup.vue:34-36,112`).
- **Look cần đọc:** trục; item nào active; item nào không focusable; item nào có focus. Container không bao giờ vẽ vòng focus (`RovingFocusGroup.vue:157`).
- **Cơ chế DOM không port:**
  - `tabindex` 0/-1 trên container và item → một Tab stop.
  - `CustomEvent` không nổi bọt, hủy được (`utils.ts:6-7`) → caller chặn được entry focus.
  - `isClickFocus` + `setTimeout(…, 1)` sau `mouseup` (`RovingFocusGroup.vue:118-123`) → phân biệt focus chuột và bàn phím.
  - Ghi Tab stop ngay ở `mousedown` (`RovingFocusItem.vue:137-138`) → Safari không phát `focus` khi click button.
  - `nextTick(() => focusFirst(...))` (`RovingFocusItem.vue:100`) → focus sau khi render xong.
  - `keydown` gắn native sau mount, gắn lại ở `focusin` (`RovingFocusItem.vue:103-119`) → handler của con `asChild` chạy trước roving.
  - Dò `data-disabled`, `data-active`, `data-highlighted` trên DOM (`RovingFocusGroup.vue:103-105`) → lọc item tắt, tìm ứng viên ưu tiên.
  - `focus()` rồi so `document.activeElement` (`utils.ts:45-55`) → biết ứng viên có nhận focus không.
  - Thuộc tính `dir` trên container (`RovingFocusGroup.vue:156`) → hướng chữ.
- **Dễ sót:**
  - Loop mặc định **tắt** (`RovingFocusGroup.vue:67`). `useArrowNavigation` mặc định bật.
  - `PageUp`/`PageDown` = đầu/cuối (`utils.ts:14,16`).
  - "Active" không đặt Tab stop. Nó chỉ thắng ở entry focus (`RovingFocusGroup.vue:104,109`). Tab stop chỉ đổi khi một item nhận focus.
  - Vào lại nhóm không đối xứng. Tab xuôi gặp container trước (container đứng trước item trong thứ tự cây) → entry focus → item active. Shift+Tab ngược gặp thẳng Tab stop (`RovingFocusGroup.vue:152`; `RovingFocusItem.vue:126`).
  - Cờ "đang Shift+Tab ra" chỉ xóa khi chính container mất focus (`RovingFocusGroup.vue:161`). `blur` không nổi bọt, nên item mất focus không xóa nó. Sau khi Shift+Tab ra, Tab xuôi vào lại rơi thẳng vào Tab stop cũ, không qua entry focus.
  - Tab stop không chuyển khi item đó thành không focusable hoặc bị gỡ: tabindex chỉ phụ thuộc "là Tab stop" (`RovingFocusItem.vue:35-37,126`). Container vẫn tabbable, nên Tab vẫn vào được qua entry focus.
  - Item highlighted thắng Tab stop hiện tại ở entry focus (`RovingFocusGroup.vue:105,109`).
- **Docs lệch nguồn:** docs ghi cờ active "có mặt khi không active" (`docs:utilities/roving-focus.md:61-64`); nguồn: có mặt khi active (`RovingFocusItem.vue:128`). Docs ghi Space/Enter "kích hoạt click" (`docs:utilities/roving-focus.md:152-155`); RovingFocus không xử lý hai phím này, đó là hành vi của button. Docs thiếu `Home`/`End`/`PageUp`/`PageDown` và luật trục.

### Collection
`Collection/` · dùng bởi: `RovingFocus` (nên mọi primitive ở trên), `Listbox` (→ `Combobox`, `Autocomplete`), `Select`, `Menu`, `Menubar`, `NavigationMenu`, `Slider`, `TagsInput`, `Toast`, `Tree` → shadcn-vue `select`, `combobox`, `command`, `navigation-menu`, `slider`, `tags-input` và các component của RovingFocus.

- **Vai trò:** root biết danh sách item của nó theo đúng thứ tự hiển thị, kèm dữ liệu của từng item.
- **Trạng thái:** sở hữu tập item đã đăng ký: node + dữ liệu tùy ý (giá trị, nhãn cho typeahead). Chỉ cấu trúc ghi: item mount → đăng ký; node đổi hoặc unmount → hủy (`Collection.ts:86-92`). User và caller không ghi. Root đọc: danh sách có thứ tự (mặc định bỏ item tắt), hoặc tra item theo node (`:31-54`).
- **Bảo đảm:**
  - Thứ tự = thứ tự trong cây lúc hỏi, không phải thứ tự đăng ký (`Collection.ts:39-48`; test `Collection/Collection.test.ts:93-114`). Item chèn vào giữa nằm đúng chỗ ngay lần hỏi sau.
  - Mặc định bỏ item tắt. Xin cả item tắt thì vẫn đúng thứ tự (`:50-53`; test `:116-133`).
  - Item unmount biến mất ở lần hỏi sau (`:91`; test `:135-156`).
  - Root chưa có node → danh sách rỗng (`:40-42`; test `:195-218`).
  - Collection lồng nhau tách biệt. Item thuộc root gần nhất cùng khóa. Khóa khác (vd. của `Menubar`, `NavigationMenu`) tách hẳn với collection bên trong nó (`:12-14,28`; `Menubar/MenubarRoot.vue:67`; `NavigationMenu/NavigationMenuRoot.vue:137`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:**
  - `querySelectorAll('[data-reka-collection-item]')` mỗi lần hỏi (`:43-48`) → thứ tự hiển thị.
  - `Map` khóa bằng `HTMLElement` + `markRaw` (`:88-91`) → tra item theo node.
  - Lọc `data-disabled=""` (`:36,53`) → bỏ item tắt.
  - `CollectionSlot`/`CollectionItem` gắn thuộc tính lên node của caller qua `Slot` (`:56-97`) → caller giữ quyền chọn node.
  - `provide`/`inject` theo chuỗi khóa (`:14,25,28`) → root gần nhất.
- **Dễ sót:**
  - Item nằm ngoài cây của root (vd. đã teleport) xếp **đầu** danh sách (chỉ số -1) (`:47`).
  - Item tách khỏi cây mà chưa unmount vẫn tra thấy (test `Collection/Collection.test.ts:177-193`).
  - Danh sách reactive reka cũng xuất ra theo thứ tự đăng ký, không theo thứ tự hiển thị (`:99-100`).
  - Chỉ biết item đã mount. Danh sách ảo hóa tự làm typeahead và Home/End trên dữ liệu (`Listbox/ListboxVirtualizer.vue:313-341`).

### Primitive (`as`, `asChild`, `Slot`)
`Primitive/` · dùng bởi: mọi part của reka. shadcn-vue dùng trực tiếp ở `Button`, `Badge`, `BreadcrumbLink`, `ButtonGroupText`, `Item`, `SidebarMenuButtonChild`, `SidebarMenuAction`… (36 file); `Slot` ở `FormControl` (`shadcn:form/FormControl.vue:9-16`).

- **Vai trò:** một part mang hành vi và ngữ nghĩa của nó lên node do caller chọn.
- **Trạng thái:** không state. Caller chọn một lần: part tự render node mặc định của nó (mỗi part có loại riêng), đổi loại node, hoặc gộp vào con đầu tiên của caller (`Primitive.ts:105-119`).
- **Bảo đảm:**
  - Gộp vào con: con nhận đủ ngữ nghĩa, ARIA và handler của part (`Slot.ts:22-29`).
  - Ưu tiên khi gộp: thuộc tính của con thắng; `class`, `style` cộng dồn; handler chạy cả hai, của part trước rồi của con (`Slot.ts:26-29`; `vue:runtime-core/src/vnode.ts:900-932`).
  - Chỉ con đầu tiên nhận (bỏ comment, làm phẳng fragment). Các con khác giữ nguyên (`Slot.ts:12-17,37-41`; `shared/renderSlotFragments.ts:4-13`).
  - Gộp nhiều tầng được: một nút vừa là trigger của Tooltip vừa của Dialog (`docs:guides/composition.md:52-77`).
  - FormControl của shadcn-vue gộp id, `aria-describedby` (mô tả + lỗi) và `aria-invalid` lên control của caller (`shadcn:form/FormControl.vue:9-16`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:**
  - `cloneVNode` + `mergeProps` (`Slot.ts:26-29`) → gộp lên node của caller.
  - Bỏ `disabled=false` trên thẻ thường (`Slot.ts:30-35`; `shared/disabledAttr.ts:6-18`) → part đang bật không bị render thành "disabled".
  - Thẻ tự đóng (`img`, `input`, `area`) không có con (`Primitive.ts:39-40,121-122`) → hydration.
  - `as` nguy hiểm (`script`, `iframe`, `object`…) hoặc object không render được → render `div` (`Primitive.ts:66-82,124-131`) → chống chèn mã.
  - Xóa `ref` của con (`Slot.ts:19-20`) → ref của part trỏ đúng node.
- **Dễ sót:**
  - Lựa chọn "tự render" hay "gộp vào con" chỉ đọc khi tạo (`Primitive.ts:119`). Đổi sau không có tác dụng.
  - Khi gộp, part không biết loại node. Node không focus được là trách nhiệm của caller (`docs:guides/composition.md:39-43`).
  - Handler của part chạy trước handler của con, nên con không giành phím được qua cơ chế gộp. RovingFocus phải gắn listener riêng để con chạy trước (`RovingFocus/RovingFocusItem.vue:103-112`).
- **Docs lệch nguồn:** docs không nói việc chặn `as` nguy hiểm và thẻ tự đóng.

### VisuallyHidden
`VisuallyHidden/VisuallyHidden.vue` · dùng bởi: `Tooltip` (mô tả cho AT, `Tooltip/TooltipContentImpl.vue:131-137`), `Toast` (thông báo, focus proxy), `NavigationMenu` (focus proxy), `DateField`/`TimeField`/`PinInput` (input gốc ẩn), mọi twin form (xem mục Tích hợp form). shadcn-vue không dùng trực tiếp; có qua `tooltip`, `navigation-menu`, `select`, `pin-input`, các form control.

- **Vai trò:** nội dung có cho AT (và có thể cho bàn phím) nhưng không hiện trên màn hình.
- **Trạng thái:** không state. Caller chọn một chế độ (`VisuallyHidden.vue:4-16,22`):
  - chỉ ẩn hình (mặc định): vẫn trong cây AT, vẫn được đọc, đặt tên được cho cha, giữ khả năng focus của nó;
  - ẩn hoàn toàn: rời cây AT và thứ tự Tab.
- **Bảo đảm:**
  - Chỉ ẩn hình: chữ bên trong được đọc và làm tên cho cha (vd. nút chỉ có icon) (`:7-10`; test `VisuallyHidden/VisuallyHidden.test.ts:31-42`).
  - Ẩn hoàn toàn: `aria-hidden="true"`, rời thứ tự Tab (`:48-50`).
  - Control form bị ẩn khỏi AT mà có tên field → tên field làm `aria-label`. `aria-label` của caller thắng (`:24-40`).
  - Không chiếm chỗ, không làm container cuộn (`:51-69`).
  - Làm focus proxy: nút vô hình nhận Tab rồi chuyển focus đi chỗ khác (`NavigationMenu/NavigationMenuTrigger.vue:172-176`; `Toast/FocusProxy.vue:13-21`).
- **Look cần đọc:** không vẽ gì; layout coi kích thước bằng 0.
- **Cơ chế DOM không port:**
  - Clip 1×1px, `position: absolute`, `margin: -1px`, `top/left: -1px` (`:51-69`) → ẩn hình mà vẫn trong cây AT, không gây cuộn.
  - `tabindex="-1"` và `data-hidden` (`:49-50`) → rời thứ tự Tab, cho CSS nhận ra.
- **Dễ sót:**
  - Mặc định là chỉ ẩn hình, không ẩn khỏi AT (`:22`).
  - Twin form mặc định là ẩn hoàn toàn (`VisuallyHiddenInput.vue:10-13`).
- **Docs lệch nguồn:** bảng prop của docs thiếu chế độ ẩn (`docs:utilities/visually-hidden.md:58-78`). Câu "mọi thứ bên trong sẽ được đọc" (`:56`) sai với chế độ ẩn hoàn toàn (`VisuallyHidden.vue:48`).

### ConfigProvider
`ConfigProvider/` · dùng bởi: mọi primitive đọc hướng (~40, vd. `Tabs`, `RadioGroup`, `Menu`, `Select`, `Slider`, `Accordion`, `Tree`); locale (`Calendar`, `DateField`, `TimeField`, `NumberField`, `ColorField`, các picker); khóa cuộn (`Dialog`, `Drawer`, `Menu`, `Popover`, `Select`, `Combobox`); nonce (viewport của `ScrollArea`, `Select`, `Combobox`; `Splitter`); đích portal (`Teleport`); nguồn id (`useId`). shadcn-vue không bọc nó; app tự đặt.

- **Vai trò:** mặc định toàn app cho các primitive con.
- **Trạng thái:** không state riêng. Chuyển giá trị caller đặt xuống cây (`ConfigProvider.vue:72-81`). Mỗi giá trị đọc theo thứ tự: giá trị đặt riêng cho primitive > provider gần nhất > mặc định cứng.
  - Hướng: mặc định `ltr` (`:64`; `shared/useDirection.ts:11-16`).
  - Locale: mặc định `en` (`:65`; `shared/useLocale.ts:10-15`).
  - Bù thanh cuộn khi khóa cuộn: mặc định bật (`:66`).
  - Nonce CSP: mặc định không có (`shared/useNonce.ts:5-10`).
  - Đích portal: mặc định `body` (`Teleport/Teleport.vue:37-39`).
  - Nguồn id: mặc định của Vue (`shared/useId.ts:26-34`).
- **Bảo đảm:**
  - Hướng `rtl` → điều hướng ngang đảo theo (`RovingFocus/utils.ts:20-28`; `shared/useArrowNavigation.ts:128`).
  - Khóa cuộn không làm nội dung phía sau xê dịch: bù đúng bề rộng thanh cuộn dọc (`shared/useBodyScrollLock.ts:62-89`).
  - Tùy chọn bù: tắt hẳn; bù vào padding phải (mặc định) hoặc margin phải; `true` = cộng thêm vào giá trị đang có, số = px, chuỗi = dùng nguyên (`:71-86`; `shared/types.ts:28-38`). Chỉ đặt margin = `true` thì padding không bù, để không bù hai lần (`:74`).
  - Không có thanh cuộn dọc → không bù (`:64`).
  - Nhiều lớp khóa chồng nhau: mở khóa khi lớp cuối nhả; giá trị cũ trả lại nguyên (`:20-26,34-47,53-56,142-158`).
- **Look cần đọc:** hướng (lật layout, icon có hướng); locale (định dạng số, ngày).
- **Cơ chế DOM không port:**
  - Padding/margin phải của `body`, `overflow: hidden`, biến CSS `--scrollbar-width` (`useBodyScrollLock.ts:62-89`) → nội dung không xê dịch.
  - Chặn `touchmove` trên iOS trừ vùng cuộn được (`:91-129,160-197`) → khóa cuộn trên iOS.
  - `nonce` trên thẻ `<style>` ẩn thanh cuộn native (`Viewport/Viewport.vue:42-50`) → chạy được dưới CSP.
  - Đích teleport (`Teleport/Teleport.vue:39`) → overlay nằm trên cùng, kể cả trong shadow DOM.
  - Nguồn id của framework (Nuxt) → id khớp giữa SSR và client.
- **Dễ sót:**
  - Provider không tự đổi hướng layout của app; nó chỉ render con (`ConfigProvider.vue:84-86`). Từng primitive tự đọc hướng.
  - Nguồn id đọc một lần, không reactive (`:80`).
  - Khóa cuộn dùng một bộ đếm chung toàn app. Nó lấy provider của primitive khóa đầu tiên, một lần (`useBodyScrollLock.ts:14,28-30`). Provider lồng với cấu hình khác không được dùng.
  - Chuỗi rỗng rơi về mặc định (so bằng `||`) (`useDirection.ts:15`; `useLocale.ts:14`).

### Label
`Label/` · dùng bởi: `FieldLabel` của reka; shadcn-vue `label`, `form` (`FormLabel`), `field` (`FieldLabel`).

- **Vai trò:** nhãn của một control.
- **Trạng thái:** không state. Caller liên kết nhãn với control bằng id của control, hoặc bọc control bên trong (`Label.vue:5-8`; `docs:components/label.md:55`).
- **Bảo đảm:**
  - Control lấy tên truy cập từ nhãn. Đây là hành vi của `<label>` native; reka không có mã riêng (`docs:components/label.md:55`).
  - Bấm nhãn = kích hoạt control (checkbox/switch đổi trạng thái, ô nhập nhận focus). Cũng là hành vi native. Vì thế `Field` có chế độ "nhãn không native": label native chuyển cả click lẫn hover sang button, vd. trigger của Select (`Field/FieldLabel.vue:5-14`).
  - Chế độ không native: bấm nhãn chỉ focus control; click trên phần tử tương tác bên trong nhãn thuộc về phần tử đó; tên đi qua `aria-labelledby` (`Field/FieldLabel.vue:38-45,54-56`).
  - Bấm hai lần trở lên không bôi đen chữ của nhãn, trừ khi caller đã chặn sự kiện (`Label.vue:24-27`).
  - Control tùy biến phải dựa trên `button` hoặc `input` thì nhãn mới hoạt động (`docs:components/label.md:55`). Control của reka mặc định là `button` (`Checkbox/CheckboxRoot.vue:73`; `Switch/SwitchRoot.vue:53`).
- **Look cần đọc:** control liên kết đang tắt hay lỗi (shadcn-vue bỏ con trỏ khi tắt, tô màu `destructive` khi lỗi: `shadcn:label/Label.vue:19`, `shadcn:form/FormLabel.vue:16-18`); chữ nhãn không chọn được.
- **Cơ chế DOM không port:**
  - Thẻ `label` + `for` (`Label.vue:14-16`) → tên truy cập và chuyển click.
  - `preventDefault` ở `mousedown` khi `event.detail > 1` (`:26`) → không chọn chữ khi nhấp nhiều lần.
- **Dễ sót:** hai hành vi chính (đặt tên, chuyển click) là của trình duyệt. reka không viết dòng nào cho chúng. S6 phải tự làm cả hai.
- **Docs lệch nguồn:** "hỗ trợ control lồng nhau" (`docs:components/label.md:21`) là hành vi trình duyệt; nguồn không có mã riêng.

### Separator
`Separator/`, `shared/component/BaseSeparator.vue` · dùng bởi: `ToolbarSeparator`, `StepperSeparator`; shadcn-vue `separator`, `command` (`CommandSeparator`), và qua `separator` cục bộ: `field`, `item`, `button-group`, `sidebar`. `MenuSeparator`, `SelectSeparator` tự làm riêng (xem Dễ sót).

- **Vai trò:** vạch phân cách, có hoặc không có ngữ nghĩa.
- **Trạng thái:** không state. Caller đặt hướng (mặc định ngang; giá trị lạ → ngang) và "chỉ trang trí" (reka mặc định không; shadcn-vue mặc định có) (`BaseSeparator.vue:24-35`; `shadcn:separator/Separator.vue:8-13`).
- **Bảo đảm:**
  - Có ngữ nghĩa: role `separator`. `aria-orientation="vertical"` chỉ khi dọc; ngang để mặc định của ARIA (`BaseSeparator.vue:36-44`).
  - Chỉ trang trí: role `none`; AT bỏ qua (`:41-43`).
  - Không focus được, không nhận phím (`:48-57`).
- **Look cần đọc:** hướng. Ngang: cao 1px, rộng hết. Dọc: rộng 1px, kéo hết chiều cao (`shadcn:separator/Separator.vue:24`).
- **Cơ chế DOM không port:** `data-orientation` (`BaseSeparator.vue:52`) → CSS chọn kiểu vạch.
- **Dễ sót:**
  - Trong shadcn-vue, separator mặc định không được đọc (trang trí) (`shadcn:separator/Separator.vue:12`).
  - reka có ba ngữ nghĩa khác nhau: `Separator` như trên; `MenuSeparator` luôn role `separator` + `aria-orientation="horizontal"` (`Menu/MenuSeparator.vue:16-17`); `SelectSeparator` `aria-hidden` (`Select/SelectSeparator.vue:15`).
  - `ToolbarSeparator` lấy nguyên hướng của toolbar (`Toolbar/ToolbarSeparator.vue:19-23`). Trong toolbar ngang, vạch đứng giữa các item nhưng báo hướng ngang.

### useTypeahead
`shared/useTypeahead.ts` · dùng bởi: `Menu` (→ `DropdownMenu`, `ContextMenu`, `Menubar`), `Select` (trigger và nội dung), `Listbox`, `Tree`, `TagGroup`; bản ảo hóa của `Listbox`/`Tree` dùng lại hàm so khớp → shadcn-vue `dropdown-menu`, `context-menu`, `menubar`, `select`.

- **Vai trò:** gõ chữ để nhảy tới item có nhãn bắt đầu bằng chuỗi đã gõ.
- **Trạng thái:** sở hữu bộ đệm tìm kiếm (chuỗi).
  - User ghi: mỗi phím in được nối thêm một ký tự (`:21,27`).
  - Đồng hồ ghi: **1000 ms** sau lần nối cuối, bộ đệm về rỗng. Mỗi lần nối khởi động lại đồng hồ (`:15-16`; `vueuse:shared dist/index.js:773-796`).
  - Caller ghi: xóa ngay (`:38-40`), vd. khi mở Select (`Select/SelectTrigger.vue:94-100`), khi Menu ẩn (`Menu/MenuContentImpl.vue:165-171`).
- **Bảo đảm (luật so khớp, `getNextMatch`, `:78-96`):**
  - So tiền tố, không phân biệt hoa thường (`:92-94`).
  - Nhãn của item = nhãn văn bản caller đặt; rỗng thì dùng chữ hiển thị đã trim (`:9-12`).
  - Tìm xuôi từ item hiện tại, qua cuối thì vòng về đầu (`:87-88`).
  - Gõ lặp một ký tự ("aaa") như gõ một lần "a": mỗi lần nhấn nhảy tới item kế bắt đầu bằng "a" (`:84-86`).
  - Chuỗi một ký tự: bỏ item hiện tại khỏi ứng viên, nên focus luôn đi tiếp (`:89-91`).
  - Chuỗi nhiều ký tự: item hiện tại còn khớp thì giữ nguyên (`:70-73,95`).
  - Không khớp, hoặc chỉ khớp item hiện tại → không đổi (`:95`).
  - Item trùng nhãn vẫn phân biệt; nhấn lặp vòng qua từng cái (`:75-77`; test `shared/useTypeahead.test.ts:67`).
  - Item hiện tại = item đang có focus. Item khớp nhận focus (`:32-35`).
- **Bảo đảm do nơi dùng đặt (composable không tự lọc phím):**
  - Chỉ phím in được (giá trị phím dài một ký tự), không kèm Ctrl/Alt/Meta (`Menu/MenuContentImpl.vue:273-274,300-301`; `Select/SelectTrigger.vue:186-194`).
  - Không chạy khi phím gõ trong ô nhập nằm bên trong (`Menu/MenuContentImpl.vue:272`; `Tree/TreeRoot.vue:182-193`, gồm cả `select` và contenteditable).
  - Select đang đóng: typeahead **chọn** luôn item khớp, tìm từ item đang chọn, như `<select>` native. Không áp dụng khi chọn nhiều (`Select/SelectTrigger.vue:83-93`).
- **Look cần đọc:** item được highlight/focus sau khi nhảy.
- **Cơ chế DOM không port:**
  - `refAutoReset` (`:16`) → xóa bộ đệm sau 1000 ms.
  - `textContent` của node làm nhãn dự phòng (`:11`) → nhãn mặc định là chữ hiển thị.
  - `getActiveElement()` xuyên shadow DOM (`:32`; `shared/getActiveElement.ts:1-12`) → item hiện tại.
- **Dễ sót:**
  - Space khi đang gõ dở. Select coi nó là ký tự tìm kiếm, không chọn (`Select/SelectTrigger.vue:185-196`; `Select/SelectContentImpl.vue:208-214`). Menu luôn bỏ Space khỏi tìm kiếm (`Menu/MenuContentImpl.vue:291-293`), và cờ "đang gõ" của Menu không bao giờ khác rỗng (`:138,170,321`; `Menu/MenuItem.vue:71-72`), nên Space luôn chọn item. Hai luật lệch nhau; S6 chọn một.
  - Bản ảo hóa so theo chuỗi, nên item trùng nhãn không vòng được: luôn về item đầu có nhãn đó (`Listbox/ListboxVirtualizer.vue:327-332`). Vẫn 1000 ms (`:250-251`).
  - Chế độ callback chỉ nối bộ đệm rồi giao phím cho caller, không focus (`:26-30`).

### useArrowNavigation
`shared/useArrowNavigation.ts` · dùng bởi: `Accordion`, `Menu` (nội dung), `NavigationMenu`, `PinInput`, `Stepper`, `TagsInput` → shadcn-vue `accordion`, `dropdown-menu`, `context-menu`, `menubar`, `navigation-menu`, `pin-input`, `stepper`, `tags-input`.

- **Vai trò:** từ một phím và item hiện tại, trả về item đích trong một danh sách. Không giữ state.
- **Trạng thái:** không state. Caller đặt mỗi lần gọi (`:85-93`): trục (mặc định cả hai), loop (mặc định **bật**), hướng (mặc định `ltr`), tiêu thụ phím (mặc định có), bỏ qua khi đang ở ô nhập (mặc định không), tự focus đích (mặc định không).
- **Bảo đảm:**
  - Phím: bốn mũi tên, `Home`, `End`. Không có `PageUp`/`PageDown` (`:95-102`).
  - Trục dọc bỏ qua Left/Right; trục ngang bỏ qua Up/Down; cả hai → nhận cả bốn (`:105-113`).
  - Xuôi = `ArrowDown`; hoặc `ArrowRight` khi `ltr`, `ArrowLeft` khi `rtl`. Up/Down không đổi theo hướng (`:128`).
  - Mũi tên bỏ qua item có `disabled` native (`:194-204`).
  - Loop tắt: ở đầu/cuối thì không có đích (`:186-187`).
  - Item hiện tại không nằm trong danh sách: xuôi → item đầu, ngược → item cuối (`:179-181`; test `shared/useArrowNavigation.test.ts:116-145`).
  - Mọi item đều tắt → không có đích, không lặp vô hạn (`:172-175`).
  - Phím hợp lệ và danh sách không rỗng → phím bị tiêu thụ, kể cả khi không có đích (`:119-123`).
  - Bật "bỏ qua ô nhập": đang ở `INPUT`/`TEXTAREA` thì không điều hướng (`:65,82-83`; dùng ở `NavigationMenu/NavigationMenuContentImpl.vue:171-176`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:**
  - `querySelectorAll` theo `data-reka-collection-item` dưới một node cha, hoặc mảng caller đưa (`:115-117`) → danh sách theo thứ tự hiển thị.
  - Thuộc tính `disabled` khác `"false"` (`:194-196`) → item tắt.
  - `e.preventDefault()` (`:122-123`) → trang không cuộn.
- **Dễ sót:**
  - `Home`/`End` **không** bỏ qua item tắt (`:134-139`). Nơi dùng phải lọc trước; Menu lọc sẵn (`Menu/MenuContentImpl.vue:181-187`).
  - Chỉ `disabled` native bị bỏ qua. Item tắt theo ARIA hay theo cờ riêng vẫn là đích (`:194-196`).
  - Loop mặc định bật, ngược RovingFocus. Phần lớn nơi dùng tắt nó (`NavigationMenu/NavigationMenuItem.vue:134-137`; `PinInput/PinInputInput.vue:119-125`; `Stepper/StepperTrigger.vue:93-99`; `TagsInput/TagsInputRoot.vue:227-231`). `Accordion` để mặc định nên vòng (`Accordion/AccordionItem.vue:101-110`).
- **Docs lệch nguồn:** không có trang docs. Chú thích trong mã sai hai chỗ: tùy chọn hướng ghi là "orientation" (`:35-40`); danh sách caller ghi "bị bỏ qua khi có attributeName", thực ra bị bỏ qua khi có node cha (`:20-26,115-117`).

### useVModel (controlled / uncontrolled)
`useVModel` của `@vueuse/core` (reka không tự viết) + `shared/useSingleOrMultipleValue.ts` · dùng bởi: mọi root có state (81 chỗ gọi): trạng thái mở của `Dialog`/`Popover`/`Menu`/`Tooltip`, giá trị của `Tabs`/`Select`/`Slider`/`Switch`/`RadioGroup`, Tab stop của `RovingFocus`… Qua đó mọi component shadcn-vue có v-model.

- **Vai trò:** một state, hai chủ có thể: primitive (uncontrolled) hoặc caller (controlled).
- **Trạng thái:**
  - Chế độ chọn **một lần khi tạo**: caller đưa giá trị (khác `undefined`) → controlled; không đưa → uncontrolled. Mọi root dùng cùng mẫu (vd. `RovingFocus/RovingFocusGroup.vue:75-78`; `Dialog/DialogRoot.vue:73-76`).
  - Controlled: giá trị luôn của caller. Hành động của user chỉ phát yêu cầu đổi. Caller không ghi lại → giá trị đứng yên (`vueuse:core dist/index.js:7871-7878`).
  - Uncontrolled: primitive giữ bản sao, khởi tạo từ giá trị mặc định caller đưa (đọc một lần) (`:7856-7858`). Hành động của user đổi bản sao và báo thay đổi (`:7867-7869`).
  - Caller vẫn đẩy được giá trị vào bản uncontrolled sau này. Giá trị được nhận, không vọng lại thành sự kiện thay đổi (`:7859-7866`).
- **Bảo đảm:**
  - "Không đưa" khác `false`/rỗng. Root khai mặc định `undefined` để tránh Vue ép boolean thiếu thành `false` (`Dialog/DialogRoot.vue:56-58`; `Switch/SwitchRoot.vue:52-55`). Lớp bọc chỉ chuyển prop caller thật sự đặt (`shared/useForwardProps.ts:28-53`).
  - Uncontrolled: chỉ báo khi giá trị đổi; ghi lại đúng giá trị cũ (cùng tham chiếu) không báo (`:7867-7868`).
  - Controlled: mỗi lần primitive xin đổi đều báo, kể cả cùng giá trị (`:7875-7877`).
  - Controlled mà caller sau đó đặt `undefined` → hiện giá trị mặc định (`:7850`). `null` là giá trị thật, không rơi về mặc định (`vueuse:shared dist/index.js:266`).
  - Một hay nhiều (`Accordion`, `ToggleGroup`, qua `useSingleOrMultipleValue`): loại caller đặt thắng; không đặt thì suy từ giá trị (mảng → nhiều); không suy được → một (`:18-33`). Mặc định: một → không có gì; nhiều → mảng rỗng (`:35-40`). Một: chọn lại giá trị đang chọn → bỏ chọn. Nhiều: bật/tắt phần tử, thêm vào cuối (`:53-68`).
  - So giá trị theo cấu trúc, không theo tham chiếu (`useSingleOrMultipleValue.ts:4,55,60`; `shared/isValueEqualOrExist.ts:13-21`).
- **Look cần đọc:** không riêng; look đọc chính state (mở/đóng, đã chọn…).
- **Cơ chế DOM không port:**
  - `computed` get/set, hoặc `ref` + hai `watch` + cờ `isUpdating` xóa ở `nextTick` (`vueuse:core dist/index.js:7856-7878`) → hai chủ cho một state, không vọng.
  - `emit('update:…')` → báo thay đổi.
  - Mặc định `undefined` chống ép boolean của Vue → phân biệt "không đưa".
- **Dễ sót:**
  - Không đổi chế độ sau khi tạo. Bắt đầu uncontrolled rồi caller đưa giá trị → vẫn là bản sao, chỉ được đẩy giá trị. Bắt đầu controlled rồi bỏ → hiện mặc định.
  - Uncontrolled báo trễ (trong watcher, trước lần render kế); controlled báo ngay (`:7867` so với `:7876`).
  - Vài root luôn uncontrolled dù caller đưa giá trị: giá trị của `Tree`, `TagsInput`, `PinInput` (`Tree/TreeRoot.vue:115-120`; `TagsInput/TagsInputRoot.vue:97-101`; `PinInput/PinInputRoot.vue:95-99`). Caller không chặn được thay đổi.
  - Chế độ nhiều: giá trị đơn falsy (`0`, `''`) bị rơi khi đổi thành mảng (`useSingleOrMultipleValue.ts:58`).
- **Docs lệch nguồn:** chú thích nói kiểu mâu thuẫn sẽ ném lỗi hoặc log; mã không ném, không log (`useSingleOrMultipleValue.ts:8-26`).

### useSelectionBehavior
`shared/useSelectionBehavior.ts` · dùng bởi: chỉ `Tree` (không có trong shadcn-vue). `Listbox` (→ `Combobox`, shadcn-vue `command`) tự viết lại luật tương tự (`Listbox/ListboxRoot.vue:155-183,393-415`).

- **Vai trò:** luật khi user chọn một item: bật/tắt (toggle) hay thay thế (replace), và chọn dải bằng Shift.
- **Trạng thái:** ghi vào giá trị của root (qua useVModel). Sở hữu thêm mốc của dải (`:9`). User ghi: chọn item (click, Enter, Space) → theo luật dưới; Shift+mũi tên → chọn dải. Caller đặt: một hay nhiều; toggle hay replace (mặc định toggle, `Tree/TreeRoot.vue:93`).
- **Bảo đảm:**
  - Nhiều + toggle: item đã chọn → bỏ; chưa chọn → thêm vào cuối. Phần tử khác giữ thứ tự (`:18-24`).
  - Nhiều + replace: chỉ còn item vừa chọn; item đó thành mốc (`:14-17`).
  - Một + toggle: chọn lại item đang chọn → bỏ chọn; item khác → chọn (`:31-36`).
  - Một + replace: luôn chọn; không bỏ chọn được (`:28-30`).
  - Dải: từ mốc tới item vừa nhận focus, gồm cả hai đầu, theo thứ tự hiển thị của các item đang mở trong cây (`:52-56`; `shared/arrays.ts:57-66`; `Tree/TreeRoot.vue:204-217`).
  - Tree: Shift+ArrowUp/Down chuyển focus trước, rồi tính dải (`Tree/TreeRoot.vue:209-216,314`; `Tree/TreeItem.vue:181`; `vue:runtime-dom/src/directives/vOn.ts:39,124-132`).
- **Look cần đọc:** item nào đang được chọn.
- **Cơ chế DOM không port:** `nextTick` rồi đọc `activeElement` (`Tree/TreeRoot.vue:209-215`) → dải tính theo item vừa nhận focus. Lọc `data-disabled` (`:45`) → item tắt không làm điểm cuối.
- **Dễ sót:**
  - Dải **chỉ** chạy khi luật là replace. Mốc chỉ được đặt ở nhánh replace (`:14-16,42-43`). Ở toggle (mặc định), Shift+mũi tên chỉ chuyển focus.
  - Mốc giữ nguyên khi kéo dải; chỉ một lần chọn replace mới đặt lại (`:14-16,67`).
  - Tree lấy dải trên mọi item đang mở, kể cả item tắt; chỉ điểm cuối được lọc (`:45-46,54`; `Tree/TreeRoot.vue:214`). Listbox lọc item tắt khỏi dải (`Listbox/ListboxRoot.vue:401,412`).
  - Chế độ một lưu **bản sao nông** của item, không phải chính nó (`:29,35`; test `shared/useSelectionBehavior.test.ts:81-106`). So sánh phải theo khóa (Tree dùng khóa, `Tree/TreeRoot.vue:244`). Chỉ đúng khi giá trị là object.
  - Listbox khác ở chi tiết: Ctrl/Meta/Alt + mũi tên (không Shift) chỉ dời highlight, không đổi chọn (`Listbox/ListboxRoot.vue:396-398`); Shift+Home/End kéo dải tới đầu/cuối (`:404-407`).

### useFilter
`shared/useFilter.ts` · dùng bởi: `Combobox`, `Autocomplete` (lọc mặc định); shadcn-vue `command` (`shadcn:command/Command.vue:24`).

- **Vai trò:** so chuỗi cho ô tìm kiếm: bắt đầu bằng, kết thúc bằng, chứa.
- **Trạng thái:** không state. Caller đưa tùy chọn so sánh kiểu `Intl.Collator`, reactive (`:19-21`).
- **Bảo đảm:**
  - Chuỗi tìm rỗng → khớp mọi thứ (`:24-25,33-34,42-43`).
  - Chuẩn hóa NFC cả hai bên: "é" dựng sẵn khớp "e" + dấu (`:27-28`; test `shared/useFilter.test.ts:61-64`).
  - So theo collator, không theo byte. Độ nhạy `base` bỏ qua hoa thường và dấu: "cafe" khớp "Café" (test `:68-73`). Combobox, Autocomplete, Command đều dùng `base` (`Combobox/ComboboxRoot.vue:209`; `Autocomplete/AutocompleteRoot.vue:157`).
  - "Chứa": so mọi cửa sổ cùng độ dài với chuỗi tìm. Chuỗi tìm dài hơn → không khớp (`:48-63`).
  - Đổi tùy chọn → kết quả lọc tính lại (`:20-21`; test `:81-106`).
- **Look cần đọc:** không (nơi dùng ẩn item không khớp và nhóm rỗng).
- **Cơ chế DOM không port:** `Intl.Collator('en', { usage: 'search' })` (`:21`) → so khớp nhận biết Unicode. S6 cần bộ so sánh tương đương.
- **Dễ sót:**
  - Locale **cố định** `en`, không đọc `ConfigProvider` (`:21`).
  - Cửa sổ cắt đúng bằng số đơn vị UTF-16 của chuỗi tìm (`:29,38,58-60`). Hai chuỗi collator coi là bằng nhưng khác độ dài thì không khớp.
- **Docs lệch nguồn:** docs gọi là "locale-aware" (`docs:utilities/use-filter.md:3,12`); locale cố định `en` (`:21`).

### useKbd
`shared/useKbd.ts` · dùng bởi: `Listbox`, `Stepper`, `Editable`, `NumberField`, `Calendar`, `RangeCalendar`, các field và picker ngày giờ → shadcn-vue `command`, `combobox`, `stepper`, `number-field`, `calendar`, `range-calendar`.

- **Vai trò:** bảng tên phím dùng chung.
- **Trạng thái:** hằng số, không state (`:1-38`).
- **Bảo đảm:** phím so theo **giá trị logic** (giá trị `KeyboardEvent.key`): `' '` là Space; `Control` (hai tên `CTRL` và `CONTROL`); `*`; `F1`–`F12`; mũi tên; `Home`, `End`, `PageUp`, `PageDown`; `Enter`, `Escape`, `Tab`, `Backspace`, `Delete`, `CapsLock`, `Shift`, `Alt`, `Meta` (`:2-35`). Một mục duy nhất theo **mã vật lý**: Space (`:36`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:** chuỗi `KeyboardEvent.key`/`code` → định danh phím.
- **Dễ sót:** nơi dùng nhận Space theo hai cách: mã vật lý (`Menu/MenuContentImpl.vue:292`; `Select/SelectContentImpl.vue:209`) và giá trị phím (`Select/SelectTrigger.vue:195`). S6 chọn một luật.

### useDirection
`shared/useDirection.ts` (cùng mẫu: `useLocale.ts`, `useNonce.ts`) · dùng bởi: ~40 primitive (`RovingFocus`, `Menu`, `Select`, `Slider`, `Tabs`, `Accordion`, `Splitter`, `Popper`, `Tree`…).

- **Vai trò:** hướng đọc hiệu lực của một primitive.
- **Trạng thái:** không state. Thứ tự: hướng caller đặt cho primitive > `ConfigProvider` gần nhất > `ltr`. Reactive (`:11-16`).
- **Bảo đảm:**
  - `rtl` đảo Left/Right trong điều hướng; không đảo Up/Down/Home/End (`RovingFocus/utils.ts:20-28`; `shared/useArrowNavigation.ts:128`).
  - Nơi dùng đảo cả hành vi ngang khác, vd. Right/Left mở/đóng nhánh cây (`Tree/TreeItem.vue:198-199`).
- **Look cần đọc:** hướng (lật layout, icon có hướng).
- **Cơ chế DOM không port:** thuộc tính `dir` trên root của primitive (vd. `RovingFocus/RovingFocusGroup.vue:156`) → hướng chữ và CSS.
- **Dễ sót:** chuỗi rỗng coi như không đặt (`:15`).
- **Docs lệch nguồn:** docs ví dụ truyền chuỗi `'ltr'` và nhận `ltr` (`docs:utilities/use-direction.md:27`). Nguồn chỉ đọc `.value` của ref (`:11,15`): chuỗi bị bỏ qua, kết quả là hướng của provider. `useLocale` lệch y hệt (`docs:utilities/use-locale.md:27`; `useLocale.ts:10,14`).

### useId
`shared/useId.ts` · dùng bởi: 46 file: id liên kết ARIA giữa các part (trigger ↔ content, label ↔ control), id Tab stop của RovingFocus. shadcn-vue `form` (`FormItem`), `command`, `chart`, `sidebar`.

- **Vai trò:** id duy nhất, ổn định cho mỗi instance.
- **Trạng thái:** không state. Thứ tự nguồn: id caller đưa (trả nguyên, không thêm tiền tố) > nguồn id của `ConfigProvider` > `useId` của Vue > bộ đếm toàn cục (`:15-34`). Id sinh có tiền tố `reka-` (`:36`).
- **Bảo đảm:**
  - Caller đặt id thì mọi liên kết ARIA dùng id đó (vd. `RovingFocus/RovingFocusItem.vue:33-34`).
  - Id sinh một lần khi tạo instance và giữ suốt đời instance (gọi trong setup, vd. `RovingFocus/RovingFocusItem.vue:33`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:** chuỗi id DOM và nguồn ổn định qua SSR/hydration (`:22-31`) → liên kết `aria-*`, `for` qua id. S6 có định danh node riêng.
- **Dễ sót:** id rỗng coi như không đưa (`:16`).
- **Docs lệch nguồn:** docs ghi "sinh id ngẫu nhiên" (`docs:utilities/use-id.md:3,15`); nguồn tất định (bộ đếm hoặc Vue) (`:29-34`).

### useForwardExpose
`shared/useForwardExpose.ts` · dùng bởi: ~260 file của reka (hầu hết part); shadcn-vue `resizable` (`ResizablePanel`).

- **Vai trò:** tham chiếu của caller tới một part trỏ tới đúng node part render, kèm method part công bố.
- **Trạng thái:** không state cho user. Caller đọc: prop của part, method part công bố, node gốc (`:46-78`).
- **Bảo đảm:**
  - Part bọc part khác vẫn trả node thật và gộp method của part bên trong (`:88-119`).
  - Node đổi (vd. con `asChild` đổi qua `v-if`) → tham chiếu cập nhật (`:21-28`).
  - Node bị gỡ → tham chiếu xóa, không giữ node cũ (`:88-97`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:** `instance.exposed`, `$el`, `triggerRef` sau `onUpdated`, giải node của fragment/comment (`:15-122`; `shared/resolveElement.ts:15-25`) → Vue không tự cho `$el` đúng với component nhiều gốc hay `asChild`. Cây retained của S6 có định danh node sẵn.
- **Dễ sót:** không có bảo đảm cho user; chỉ cho caller.

### useForwardPropsEmits (+ `useForwardProps`, `useEmitAsProps`)
`shared/useForwardPropsEmits.ts`, `useForwardProps.ts`, `useEmitAsProps.ts` · dùng bởi: 29 file reka (lớp bọc, vd. `DropdownMenu` → `Menu`); shadcn-vue 69 component dùng `useForwardPropsEmits` (`Dialog`, `Select`, `Tabs`, `Slider`, `Switch`…) và 71 dùng `useForwardProps`.

- **Vai trò:** lớp bọc chuyển nguyên prop và sự kiện xuống primitive bên trong.
- **Trạng thái:** không state. Chỉ chuyển prop mà caller thật sự đặt, hoặc lớp bọc khai mặc định; bỏ prop có giá trị `undefined` (`useForwardProps.ts:28-53`). Mọi sự kiện khai báo được nối lại nguyên vẹn (`useEmitAsProps.ts:20-37`).
- **Bảo đảm:**
  - Prop boolean không đặt **không** thành `false` khi qua lớp bọc (`useForwardProps.ts:10-19,43-52`). Nhờ đó primitive bên trong giữ chế độ uncontrolled và mặc định riêng của nó (vd. `Dialog` mặc định modal, `Dialog/DialogRoot.vue:59`).
  - Mặc định khai ở lớp bọc thắng mặc định bên trong (shadcn-vue đặt separator là trang trí, `shadcn:separator/Separator.vue:11-12`).
  - Mọi sự kiện của primitive tới caller của lớp bọc, cùng tham số (`useEmitAsProps.ts:32-34`).
- **Look cần đọc:** không.
- **Cơ chế DOM không port:** `vm.vnode.props` + `camelize` + `toHandlerKey` (`useForwardProps.ts:29-53`; `useEmitAsProps.ts:20-34`) → Vue không forward emit và ép boolean thiếu thành `false`.
- **Dễ sót:** luật "không đặt ≠ false/rỗng" phải giữ qua **mọi** tầng bọc. Một tầng làm mất nó → primitive thành controlled với `false` và không mở/đổi được nữa.

### useStateMachine
`shared/useStateMachine.ts` · dùng bởi: `Presence` (mọi nội dung có animation vào/ra: `Dialog`, `Popover`, `Tooltip`, `Menu`, `Accordion`, `Collapsible`…), thanh cuộn của `ScrollArea`.

- **Vai trò:** máy trạng thái hữu hạn: (trạng thái, sự kiện) → trạng thái kế.
- **Trạng thái:** sở hữu một trạng thái. Nơi dùng phát sự kiện. Sự kiện không có trong bảng của trạng thái hiện tại → giữ nguyên (`:34-41`).
- **Bảo đảm (bảng của nơi dùng):**
  - `Presence`: mounted —UNMOUNT→ unmounted; mounted —ANIMATION_OUT→ unmountSuspended; unmountSuspended —MOUNT→ mounted, —ANIMATION_END→ unmounted; unmounted —MOUNT→ mounted. Trạng thái đầu theo "có mặt" lúc tạo (`Presence/usePresence.ts:24,28-40`).
  - Thanh cuộn kiểu "scroll" của `ScrollArea`: hidden —SCROLL→ scrolling; scrolling —SCROLL_END→ idle, —POINTER_ENTER→ interacting; interacting —SCROLL→ interacting, —POINTER_LEAVE→ idle; idle —HIDE→ hidden, —SCROLL→ scrolling, —POINTER_ENTER→ interacting. Hiện khi khác hidden (`ScrollArea/ScrollAreaScrollbarScroll.vue:24-43`).
- **Look cần đọc:** Presence: đang hiện / đang chạy animation ra / đã gỡ. Thanh cuộn: hiện hay ẩn.
- **Cơ chế DOM không port:** không có; chỉ là `ref` + bảng tra (`:28-47`).
- **Dễ sót:** sự kiện sai trạng thái bị nuốt im lặng, không báo lỗi (`:37`).

### useSize
`shared/useSize.ts` · dùng bởi: `Popper` (kích thước mũi tên, `Popper/PopperContent.vue:254`), `SliderThumb` (kích thước thumb, `Slider/SliderThumbImpl.vue:33-36`) → shadcn-vue `popover`, `tooltip`, `hover-card`, `dropdown-menu`, `select`…, `slider`.

- **Vai trò:** kích thước border-box của một node, cập nhật khi đổi.
- **Trạng thái:** sở hữu kích thước; chỉ layout ghi. Chưa đo → 0 × 0 (`:6-8`).
- **Bảo đảm:**
  - Có kích thước ngay khi mount, trước lần định vị đầu tiên (`:15-16`).
  - Cập nhật theo border-box mỗi lần node đổi kích thước (`:18-51`).
  - Ngừng theo dõi khi unmount (`:60-63`).
- **Look cần đọc:** kích thước mũi tên (đặt vị trí, bù khoảng cách); kích thước thumb (giữ thumb trong track).
- **Cơ chế DOM không port:** `offsetWidth/offsetHeight` + `ResizeObserver` (`borderBoxSize`) (`:16,18-51`) → biết kích thước. Layout của S6 có sẵn kích thước.
- **Dễ sót:** giá trị đầu là số nguyên (đã làm tròn), các lần sau có phần lẻ (`:16` so với `:31-38`).

### Tích hợp form (`VisuallyHiddenInput`, `VisuallyHiddenInputBubble`, `useFormControl`, `BubbleSelect`)
`VisuallyHidden/VisuallyHiddenInput.vue`, `VisuallyHidden/VisuallyHiddenInputBubble.vue`, `shared/useFormControl.ts`, `Select/BubbleSelect.vue` · dùng bởi: `Checkbox`, `CheckboxGroup`, `Switch`, `Toggle`, `ToggleGroup`, `RadioGroup`, `Slider`, `Listbox`, `TagsInput`, `NumberField`, `PinInput`, `Editable`, `ColorArea`, `ColorField`, `ColorSlider`; `Select` qua `BubbleSelect`; `DateField`/`TimeField` dùng input ẩn riêng → shadcn-vue `checkbox`, `switch`, `toggle`, `toggle-group`, `radio-group`, `slider`, `select`, `command`, `tags-input`, `number-field`, `pin-input`.

- **Vai trò:** control tùy biến góp giá trị vào form khi submit, chặn submit khi bắt buộc mà rỗng, báo thay đổi cho form.
- **Trạng thái:** không state riêng; phản chiếu state của control. Caller đặt: tên field, bắt buộc, tắt. Control chỉ tham gia form khi nằm trong một form **và** có tên (vd. `Switch/SwitchRoot.vue:126`; `Slider/SliderRoot.vue:207`).
- **Bảo đảm:**
  - Cặp gửi đi (`VisuallyHiddenInput.vue:22-46`; test `VisuallyHidden/VisuallyHiddenInput.test.ts:95-114`):
    - giá trị đơn → `tên=giá-trị` (`0` → `"0"`, `false` → `"false"`);
    - `null`, `undefined`, `''` → `tên=` (một ô rỗng) (test `:28-55`);
    - mảng → `tên[0]`, `tên[1]`…; phần tử object → `tên[i][khóa]`; phần tử `null` → `tên[i]=` (`:29-37`; test `:69-82`);
    - object → `tên[khóa]`, chỉ một tầng (`:40-43`);
    - mảng rỗng → không gửi gì; nếu bắt buộc → một ô `tên=` rỗng để chặn submit (`:15-20,50-57`; test `:110-114`).
  - Control bật/tắt (checkbox, switch, toggle, radio): chỉ gửi khi đang bật. Giá trị gửi mặc định `on`, như checkbox native (`Switch/SwitchRoot.vue:55,125-134`; `Checkbox/CheckboxRoot.vue:72,331-340`). Toggle gửi `true` (`Toggle/Toggle.vue:105-114`).
  - Bật/tắt không ghi đè giá trị gửi đi (`VisuallyHiddenInputBubble.vue:36-38`; test `VisuallyHiddenInput.test.ts:116-128`).
  - Bắt buộc mà rỗng → form không hợp lệ, không submit; giá trị rỗng vẫn gửi `tên=` (test `VisuallyHidden/VisuallyHiddenInput.browser.test.ts:12-22,34-85`).
  - Tắt → twin bị `disabled` (`Switch/SwitchRoot.vue:129`), nên trình duyệt không gửi và không kiểm tra nó.
  - Mỗi lần giá trị đổi, form nhận `input` rồi `change` (nổi bọt), kể cả khi giá trị về rỗng (`VisuallyHiddenInputBubble.vue:30-51`; test `VisuallyHiddenInput.test.ts:39-55`).
  - Twin rời cây AT và thứ tự Tab; có tên field thì tên đó làm nhãn của nó (`VisuallyHidden.vue:36-40,47-50`).
  - Focus rơi vào twin (vd. trình duyệt báo lỗi ràng buộc) → chuyển sang phần nhìn thấy đầu tiên: `PinInput`, `DateField` (`PinInput/PinInputRoot.vue:171`; `DateField/DateFieldRoot.vue:396`).
  - `CheckboxGroup` bắt buộc, rỗng, không có tên vẫn chặn submit (`Checkbox/CheckboxGroupRoot.vue:79,105-111`).
  - `Select`: twin là `<select>`, nên autofill của trình duyệt điền được và đổi giá trị của Select (`Select/BubbleSelect.vue:37-57`).
  - Reset form: reka **không** đưa control tùy biến về giá trị mặc định. `Form` chỉ xóa touched/dirty/lỗi của `Field` (`Form/FormRoot.vue:170-172,189`; `Field/FieldRoot.vue:441-455`; test `Form/Form.test.ts:520-546`). Không có listener `reset` nào khác trong `packages/core/src`.
- **Look cần đọc:** không (twin không vẽ). Trạng thái lỗi, bắt buộc thuộc `Field`.
- **Cơ chế DOM không port:**
  - `<input>`/`<select>` ẩn cạnh control, mang tên, giá trị, trạng thái bật, bắt buộc, tắt (`VisuallyHiddenInputBubble.vue:54-59`; `Select/SelectRoot.vue:242-263`) → form native thu giá trị và kiểm tra ràng buộc.
  - Gọi setter `value`/`checked` của prototype rồi `dispatchEvent` (`VisuallyHiddenInputBubble.vue:30-51`; `Select/BubbleSelect.vue:23-35`) → sự kiện tới form như do user gây ra.
  - `closest('form')` (`useFormControl.ts:5-8`) → chỉ render twin khi nằm trong form; chưa có node (SSR) → coi là có.
  - Twin đặt cạnh, không lồng trong control (`Checkbox/CheckboxRoot.vue:201-203`) → tránh lỗi a11y "nested-interactive".
  - `<select>` không mang attribute `value` (`BubbleSelect.vue:49-50`) → giá trị đặt bằng code vẫn báo `change`.
  - Form của reka có `novalidate` (`Form/FormRoot.vue:187`) → popup kiểm tra của trình duyệt không che lỗi của Field.
- **Dễ sót:**
  - Không có tên → không tham gia form (trừ `CheckboxGroup` bắt buộc).
  - `null`/`undefined` thành chuỗi rỗng, không phải `"undefined"`; rỗng phải trượt "bắt buộc" (`VisuallyHiddenInputBubble.vue:44-47`).
  - Control mang giá trị gửi ô rỗng (`tên=`); control bật/tắt khi tắt không gửi gì.
  - Mảng rỗng + bắt buộc → một ô; mảng rỗng không bắt buộc → không ô nào.
  - Reset lệch: Vue đặt `value`/`checked` của twin là DOM property, không phải attribute mặc định (`vue:runtime-dom/src/patchProp.ts:84-160`; `vue:runtime-dom/src/modules/props.ts:28-47`). Reset native đưa twin về rỗng/tắt trong khi control giữ giá trị; hai bên lệch tới lần đổi kế. Đây là suy luận; reka không có test cho nó.
- **Docs lệch nguồn:** docs nói reset native xóa touched/dirty/lỗi của mọi Field (`docs:components/form.md:24`) — đúng, nhưng không nói giá trị của control tùy biến không được reset.

---

### Ghi chú chung cho nhóm 2
- Thứ tự item luôn là thứ tự hiển thị lúc hỏi, không phải thứ tự đăng ký (`Collection/Collection.ts:39-48`).
- reka có ba khái niệm "tắt": cờ tắt của item (Collection, RovingFocus, useSelectionBehavior lọc theo nó); `disabled` native (useArrowNavigation chỉ bỏ qua cái này); không gì cả (Home/End của useArrowNavigation) (`Collection.ts:36`; `shared/useArrowNavigation.ts:134-139,194-196`). S6 cần một khái niệm "item không nhận focus" duy nhất.
- Bảng phím không thống nhất. RovingFocus: loop tắt, có PageUp/PageDown. useArrowNavigation: loop bật, không có PageUp/PageDown (`RovingFocus/utils.ts:9-18`; `RovingFocus/RovingFocusGroup.vue:67`; `shared/useArrowNavigation.ts:89,95-102`). Mặc định phải ghi rõ theo từng primitive.
- `rtl` chỉ đảo Left/Right.
- Thứ tự xử lý phím: handler của phần tử trong cùng (con `asChild`, item) chạy trước; điều hướng bỏ qua phím đã bị chặn (`RovingFocus/RovingFocusItem.vue:62-63,103-112`). Nhờ vậy một trigger lồng trong toolbar giành được phím của nó.
- Phím gõ trong ô nhập (`input`, `textarea`, `select`, contenteditable) không chạy typeahead hay điều hướng của danh sách bao quanh (`Tree/TreeRoot.vue:182-193`; `Menu/MenuContentImpl.vue:272,300`).
- Controlled hay uncontrolled chọn một lần khi tạo. "Không đưa" phải khác `false`/rỗng qua mọi tầng bọc (useVModel, useForwardProps).
- So giá trị theo cấu trúc, không theo tham chiếu (`shared/isValueEqualOrExist.ts:13-21`; `shared/arrays.ts:57-60`).
- Typeahead: 1000 ms, phím in được, không Ctrl/Alt/Meta. Luật Space khi đang gõ khác nhau giữa Select và Menu.
- Hằng thời gian khác của nhóm là cơ chế, không phải giá trị cần giữ: xóa cờ "focus do chuột" 1 ms sau `mouseup` (`RovingFocus/RovingFocusGroup.vue:118-123`); cờ "do user" của Listbox 1 ms (`Listbox/ListboxRoot.vue:180-182`).
- Collection của reka chỉ biết item đã mount. Danh sách ảo hóa vá bằng typeahead và Home/End trên dữ liệu, và mất phân biệt item trùng nhãn (`Listbox/ListboxVirtualizer.vue:313-341`). Mục tiêu của S6 là danh sách hàng triệu dòng, nên mọi bảo đảm trên (thứ tự, bỏ item tắt, typeahead, Home/End, dải Shift) phải đúng trên toàn bộ dữ liệu, không chỉ phần đang hiển thị.
- S6 không có form native. Bộ bảo đảm của mục Tích hợp form (cặp gửi đi, bắt buộc, tắt, báo thay đổi) phải được thiết kế lại. Reset form là quyết định mới; reka không làm.
- Dùng một part ngoài root của nó → lỗi nêu tên root cần có (`shared/createContext.ts:39-47`).

## 7. Nhóm 3: overlay

### Dialog
`Dialog/` · dùng bởi: `dialog`, `sheet` (Dialog cộng với một cạnh trượt; `sidebar` dùng nó trên mobile, `shadcn:sidebar/Sidebar.vue:33`), `command` (`CommandDialog`, `shadcn:command/CommandDialog.vue:30`)

- **Part:**
  - Root: không vẽ gì. Giữ state, cấp context. Cho phép đóng bằng code từ bên trong (`DialogRoot.vue:105-110`).
  - Trigger: nút (`button`, `type=button`). Bấm thì đảo trạng thái mở (`DialogTrigger.vue:14-16,26-32,39`).
  - Portal: đưa overlay và content lên lớp trên cùng (`DialogPortal.vue:13-17`).
  - Overlay: lớp phủ phần bị trơ. Chỉ có khi modal (`DialogOverlay.vue:35`).
  - Content: `role=dialog`. Là một dismissable layer bọc trong một focus scope (`DialogContentImpl.vue:93-118`).
  - Title: heading (`h2`). Là tên truy cập (accessible name) của content (`DialogTitle.vue:12,20`, `DialogContentImpl.vue:110`).
  - Description: đoạn văn (`p`). Là mô tả truy cập (accessible description) của content (`DialogDescription.vue:12,21`, `DialogContentImpl.vue:109`).
  - Close: nút. Bấm thì đóng (`DialogClose.vue:24`).
- **Trạng thái:**
  - Mở/đóng. Mặc định đóng (`DialogRoot.vue:56-61`).
    - Người dùng ghi: Trigger đảo; Close, Esc, bấm ra ngoài hoặc focus ra ngoài thì đóng.
    - Caller ghi bất kỳ lúc nào, controlled hoặc uncontrolled, và được báo mỗi lần đổi (`DialogRoot.vue:73-76`).
    - Root không có veto. Veto nằm ở từng nguồn đóng (xem **Đóng**).
  - Modal. Mặc định bật; caller đặt (`DialogRoot.vue:59`).
    - Bật nghĩa là: bẫy focus, AT chỉ thấy dialog, bên ngoài trơ với pointer, khoá cuộn trang nếu có overlay.
    - Tắt: không có điều nào trong số đó.
  - Ngoại lệ trong modal: caller có thể để bên ngoài vẫn nhận pointer. Mặc định của modal vẫn là bên ngoài trơ (`DialogContent.vue:27-33`, `DialogContentModal.vue:11-13`).
  - Giữ nội dung khi đóng. Mặc định không: nội dung bị hủy sau animation đóng (`DialogRoot.vue:16-20,60`). Khi bật:
    - nội dung ẩn nhưng giữ state con;
    - nó rời layer stack và focus stack;
    - mỗi lần mở lại thì auto-focus lại (`DialogContent.vue:41-55`, `DialogContentImpl.vue:45-50`).
  - State nội bộ: phần tử có focus lúc mở, dùng để trả focus (`Dialog/utils.ts:49-94`).
- **Bàn phím:**

  | Phím | Kết quả |
  |---|---|
  | Enter, Space trên Trigger | Mở. Đang mở mà focus ở Trigger thì đóng. Điều này chỉ xảy ra khi non-modal, vì modal giữ focus trong content. |
  | Tab | Sang phần tử tab được kế tiếp trong content. Ở cuối thì quay về đầu, cả khi modal lẫn non-modal (`DialogContentImpl.vue:93-96`, `FocusScope/FocusScope.vue:334-365`). Không có gì tab được thì focus đứng yên (`FocusScope/FocusScope.vue:350-354`). |
  | Shift+Tab | Ngược lại. Ở đầu thì sang cuối. |
  | Esc | Đóng dialog trên cùng (`DismissableLayer/DismissableLayer.vue:146-163`). Focus trả về như mục **Focus**. |
  | Enter, Space trên Close | Đóng. |

- **Focus:**
  - Khi mở: focus phần tử tab được đầu tiên theo thứ tự tab (tabindex dương đi trước), không có thì focus chính content. Nếu đó là ô nhập thì chọn hết chữ. Không cuộn trang (`FocusScope/FocusScope.vue:225-237`, `FocusScope/utils.ts:232-247`, `Dialog/Dialog.test.ts:108-115`). Caller chặn được (reka `openAutoFocus`, `DialogContentImpl.vue:98`).
  - Bẫy focus: modal bẫy trong lúc mở và nhả ngay khi bắt đầu đóng (`DialogContentModal.vue:61`). Non-modal không bẫy (`DialogContentNonModal.vue:44`).
  - Khi đóng, focus về phần tử có focus lúc mở:
    - Nếu mở bằng Trigger thì về Trigger, kể cả khi cú bấm không đưa focus tới nó (vd focus đang bị một dialog khác bẫy) (`Dialog/utils.ts:56-66`, `DialogTrigger.vue:26-31`).
    - Phần tử đó đã rời cây thì về Trigger (`Dialog/utils.ts:88-94`).
  - Không trả focus khi:
    - trong lúc đóng, thứ khác đã lấy focus, vd một layer mở ra bởi chính cú bấm đó (`DialogContentModal.vue:63-71`, `FocusScope/FocusScope.vue:286-291`);
    - dialog non-modal và người dùng đã tương tác bên ngoài (`DialogContentNonModal.vue:46-58`).
  - Caller chặn được việc trả focus (reka `closeAutoFocus`).
  - Mở lại trong lúc đang animate đóng: giữ bản ghi focus của lần mở trước (`Dialog/utils.ts:76-79`).
- **Pointer:**
  - Trigger phản ứng ở `click` (lúc nhả), không ở pointerdown (`DialogTrigger.vue:44`).
  - Bấm ra ngoài: chuột và bút tính ở pointerdown. Màn chạm thì đợi `click`, nên cuộn hoặc kéo trên màn chạm không làm đóng (`DismissableLayer/utils.ts:130-151`).
  - Modal:
    - chuột phải hoặc Ctrl+click trái ở ngoài không đóng (`DialogContentModal.vue:72-83`);
    - bên ngoài trơ: cú bấm ra ngoài đầu tiên chỉ đóng dialog, không kích hoạt thứ nằm dưới (`DismissableLayer/DismissableLayer.vue:19-24`).
  - Non-modal: bấm Trigger lúc đang mở thì đóng một lần, không đóng rồi mở lại. Phần tử đã mở dialog bằng code cũng được đối xử như vậy (`DialogContentNonModal.vue:67-72`, `Dialog/DialogReturnFocus.browser.test.ts:214-215`).
- **Đóng:**
  - Esc. Chỉ layer trên cùng nhận. Caller chặn được (reka `escapeKeyDown`).
  - Bấm ra ngoài. Caller chặn được (reka `pointerDownOutside`, `interactOutside`). Vd của docs: bấm vào một toaster toàn cục (vue-sonner) không làm đóng dialog, vì caller chặn khi đích nằm trong toaster (`docs:components/dialog.md:284-288`, demo `references/reka-ui/docs/components/demo/DialogToaster/tailwind/index.vue:29-32`).
  - Focus ra ngoài.
    - Modal: không bao giờ đóng (`DialogContentModal.vue:84-90`).
    - Non-modal: đóng. Caller chặn được (reka `focusOutside`, `interactOutside`).
  - Close, Trigger (đảo), caller, lệnh đóng từ bên trong.
  - Các trường hợp không tính là "bên ngoài":
    - tương tác trong một layer con mở sau, vd popover trong dialog (`DismissableLayer/utils.ts:27-68`);
    - tương tác trong layer "nhánh", vd toast viewport (`DismissableLayer/DismissableLayer.vue:119-124,133-138`).
  - Dialog lồng nhau: Esc và bấm ra ngoài chỉ đóng dialog con.
  - Cú bấm mở dialog không tự làm nó đóng: reka chỉ bắt đầu nghe pointerdown ngoài sau một tick (`DismissableLayer/utils.ts:160-176`).
- **ARIA:**
  - Trigger: `aria-haspopup=dialog` và `aria-expanded`. `aria-controls` trỏ tới content chỉ khi đang mở và content đã có thật (`DialogTrigger.vue:40-42`, `shared/useTriggerId.ts:208-218`).
  - Content: `role=dialog`. `aria-labelledby` trỏ tới Title. `aria-describedby` trỏ tới Description. Caller có thể bỏ description (`DialogContentImpl.vue:108-110`, `docs:components/dialog.md:149`).
  - Modal: AT chỉ thấy dialog (`DialogContentModal.vue:37-38`). reka làm bằng cách ẩn mọi thứ khác khỏi AT, không đặt `aria-modal` (grep `src/` không thấy).
- **Look cần đọc:**
  - Mở/đóng cho trigger, overlay và content, kể cả trong lúc animate đóng (`DialogTrigger.vue:43`, `DialogOverlayImpl.vue:34`, `DialogContentImpl.vue:111`).
  - `sheet` cần thêm cạnh (trên, phải, dưới, trái; mặc định phải). Đó thuần là look (`shadcn:sheet/SheetContent.vue:27`).
- **RTL:** không đổi gì. Không đọc `dir`.
- **Cơ chế DOM không port:**
  - Teleport → nội dung vẽ trên mọi thứ, không bị cha cắt.
  - `aria-hidden` lên mọi phần tử anh em (`shared/useHideOthers.ts:11-46`) → AT chỉ thấy dialog khi modal.
  - Khoá cuộn body kèm bù scrollbar, giữ tới khi content rời hẳn (`DialogContentModal.vue:18-31`, `DialogOverlayImpl.vue:19-25`) → trang nền không cuộn và không xô lệch.
  - `pointer-events: none` trên body, `auto` trên layer cao nhất → bên ngoài trơ khi modal.
  - `pointerdown.left.self.prevent` trên overlay (`DialogOverlayImpl.vue:36`) → bấm lớp nền không cướp focus, không bôi chữ.
  - Đếm content đã render, dùng MutationObserver (`shared/useTriggerId.ts:108-206`) → `aria-controls` không trỏ vào id chưa tồn tại.
  - `setTimeout(0)` trước khi nghe pointerdown; đợi `click` trên màn chạm → cú bấm mở không tự đóng; cuộn bằng tay không đóng.
  - `v-show` cộng cờ `present` (`DialogContent.vue:59-73`) → giữ state con khi đóng.
  - Bẫy focus bằng focusin/focusout và MutationObserver (`FocusScope/FocusScope.vue:104-219`) → focus không thoát ra ngoài.
  - Bỏ qua focusin của Safari theo sau pointerdown ngoài (`DialogContentNonModal.vue:74-80`) → một cú bấm ra ngoài chỉ tính một lần.
- **Dễ sót:**
  - Docs viết Esc "moves focus to `DialogTrigger`" (`docs:components/dialog.md:369-371`). Source trả focus về phần tử có focus lúc mở; Trigger chỉ là phương án dự phòng (`Dialog/utils.ts:88-94`). Source thắng.
  - Cảnh báo lúc dev:
    - thiếu Title thì luôn cảnh báo;
    - thiếu Description thì cảnh báo, trừ khi caller đã bỏ describedby (`Dialog/utils.ts:18-47`, `DialogContentImpl.vue:80-89`).
  - Title bị ẩn khỏi mắt vẫn hợp lệ. `sidebar` mobile và `CommandDialog` dùng Title/Description chỉ dành cho AT (`shadcn:sidebar/Sidebar.vue:44`, `shadcn:command/CommandDialog.vue:30`).
  - Non-modal vẫn vòng Tab trong content. Tab không đưa focus ra khỏi dialog. Chỉ pointer hoặc code làm được (`DialogContentImpl.vue:95`).
  - Khoá cuộn đi theo overlay. Dialog modal không có overlay thì trang vẫn cuộn (`DialogContentModal.vue:18-31`, `DialogRoot.vue:37`).
  - Một dialog đóng nhưng còn giữ nội dung không nuốt Esc của dialog đang hiện, vì nó đã rời layer stack (`DismissableLayer/DismissableLayer.vue:146-152,197-213`, `Dialog/Dialog.test.ts:433`).

---

### AlertDialog
`AlertDialog/` · dùng bởi: `alert-dialog`

Là Dialog luôn ở chế độ modal (`AlertDialogRoot.vue:8,22-27`). Dưới đây chỉ ghi chỗ khác Dialog.

- **Part:**
  - Root, Trigger, Portal, Overlay, Title, Description: như Dialog.
  - Content: `role=alertdialog` (`AlertDialogContent.vue:50`).
  - Cancel: nút đóng. Tự đăng ký làm đích focus khi mở (`AlertDialogCancel.vue:17-23`).
  - Action: nút đóng (chính là Close, `AlertDialogAction.vue:15-18`). Hành động thật do caller làm.
- **Trạng thái:**
  - Mở/đóng: như Dialog.
  - Không chỉnh được chế độ modal; luôn bật.
  - Giữ nội dung khi đóng: như Dialog.
- **Bàn phím:**

  | Phím | Kết quả |
  |---|---|
  | Enter, Space trên Trigger | Mở. |
  | Tab, Shift+Tab | Vòng trong content. |
  | Esc | Đóng. Không bị chặn. |
  | Enter, Space trên Cancel hoặc Action | Đóng. |

- **Focus:**
  - Khi mở: focus Cancel ở tick sau, không cuộn trang (`AlertDialogContent.vue:53-61`). Không có Cancel thì focus phần tử tab được đầu tiên, vì focus scope vẫn chạy.
  - Khi đóng: như Dialog modal.
- **Pointer:** bấm lớp nền hoặc bất cứ đâu bên ngoài đều không đóng (`AlertDialogContent.vue:51-52`).
- **Đóng:**
  - Đóng bằng: Esc, Cancel, Action, caller.
  - Không bao giờ đóng khi bấm ra ngoài hay focus ra ngoài: mọi sự kiện "bên ngoài" đều bị chặn (`AlertDialogContent.vue:51-52`).
- **ARIA:** như Dialog, nhưng role là `alertdialog`.
- **Look cần đọc:** như Dialog.
- **RTL:** không.
- **Cơ chế DOM không port:**
  - Như Dialog.
  - `nextTick` cộng `focus({ preventScroll })` → Cancel nhận focus sau khi focus scope chạy xong, và không cuộn trang.
- **Dễ sót:**
  - Không đóng khi bấm ra ngoài (`AlertDialogContent.vue:51-52`).
  - Focus vào Cancel, không vào phần tử tab được đầu tiên (`AlertDialogContent.vue:53-61`, `AlertDialog/AlertDialog.test.ts:43`).
  - Caller chặn auto-focus lúc mở cũng không ngăn được việc focus vào Cancel, vì handler không xét `defaultPrevented` (`AlertDialogContent.vue:53-61`). Dialog thì chặn được. Có vẻ là lỗi. S6 nên cho chặn và ghi lại chỗ khác.
  - Có nhiều Cancel: Cancel đã đăng ký giữ quyền khi một Cancel khác unmount (`AlertDialogContent.vue:36-44`, `AlertDialog/AlertDialog.test.ts:91`).
  - Cảnh báo thiếu title vẫn dùng tên `DialogContent`/`DialogTitle` (`DialogContentImpl.vue:80-89`).
  - Docs viết Esc đưa focus về Trigger (`docs:components/alert-dialog.md:247`). Cùng sai lệch như Dialog.

---

### Drawer
`Drawer/` · dùng bởi: `drawer`.

shadcn dùng Drawer của reka, không dùng vaul. `DrawerContent` của shadcn luôn kèm overlay và handle; handle chỉ hiện khi drawer ở cạnh dưới (`shadcn:drawer/DrawerContent.vue:28,44`). Drawer của reka là bản port từ Base UI.

- **Part:**
  - Root: giữ state. Đọc context của drawer cha nếu có, để hỗ trợ lồng nhau (`DrawerRoot.vue:159-162`).
  - Trigger: nút. Bấm thì đảo (`DrawerTrigger.vue:13,41`).
  - Portal.
  - Overlay: lớp nền. Chỉ có khi chế độ modal khác "tắt". Drawer lồng không có overlay riêng, trừ khi caller ép (`DrawerOverlay.vue:24,30`).
  - Content: `role=dialog`. Là layer, focus scope và vùng kéo. Kéo được từ bất kỳ chỗ nào trên content (`DrawerContentImpl.vue:436-464`).
  - Handle: tay nắm trang trí, ẩn khỏi AT (`DrawerHandle.vue:20`).
  - Title, Description: như Dialog (`DrawerTitle.vue:12,20`, `DrawerDescription.vue:12,20`).
  - Close: nút đóng (`DrawerClose.vue:21`).
  - SwipeArea: vùng ở mép để vuốt **mở**. Hướng vuốt ngược với hướng đóng. Tắt khi drawer đang mở (`DrawerSwipeArea.vue:35-45`).
  - Viewport: khung đo cho bàn phím ảo (`DrawerViewport.vue:13-38`).
  - VirtualKeyboardProvider: giữ ô nhập đang có focus không bị bàn phím ảo che (`DrawerVirtualKeyboardProvider.vue:16-23`).
  - Provider, Indent, IndentBackground: trang nền co lại, lùi ra sau khi có drawer mở, kiểu iOS (`DrawerProvider.vue:53-82`, `DrawerIndent.vue:20-57`).
- **Trạng thái:**
  - Mở/đóng. Mặc định đóng.
    - Mỗi yêu cầu đổi kèm một lý do: vuốt, Esc, bấm ngoài, bấm Trigger, bấm Close (`DrawerRoot.vue:12-19,241-263`).
    - Caller nghe và có thể từ chối mọi yêu cầu. Bị từ chối thì drawer giữ nguyên; đang vuốt dở thì trượt về chỗ cũ (`DrawerRoot.vue:245-259`, `DrawerContentImpl.vue:176-188`).
    - Drawer controlled mà caller không cập nhật state cũng trượt về (`Drawer/Drawer.swipe.test.ts:156`).
  - Đã ổn định. Caller được báo khi animation mở hoặc đóng đã chạy xong, không phải lúc yêu cầu (`DrawerRoot.vue:57-62,184-188`, `DrawerContentImpl.vue:318-342,408-421`, `Drawer/utils.ts:213-251`).
    - Bị ngắt giữa chừng thì không báo.
    - Root unmount thì không báo.
    - Animation vô hạn thì không đợi.
  - Cạnh, cũng là hướng vuốt để đóng: dưới (mặc định), trên, trái, phải. Caller đặt (`DrawerRoot.vue:40-41,117`).
  - Chế độ modal, 3 mức, mặc định bật (`DrawerRoot.vue:33-39`, `DrawerContent.vue:42-50`):
    - bật: bẫy focus, AT chỉ thấy drawer, bên ngoài trơ, có overlay và khoá cuộn trang;
    - chỉ bẫy focus: pointer vẫn tới được bên ngoài;
    - tắt: không có điều nào.
  - Snap point (`DrawerRoot.vue:42-52,136-140`):
    - danh sách snap point do caller đặt;
    - snap đang chọn có hai chiều: người dùng đổi bằng cách kéo, caller ghi được;
    - không chọn snap nào thì drawer mở hết chiều cao.
  - Snap tuần tự. Mặc định tắt. Bật thì mỗi lần thả chỉ đi một nấc (`DrawerRoot.vue:47-52`).
  - Đích focus khi mở: mặc định phần tử tab được đầu tiên. Caller có thể chọn "không focus" hoặc một phần tử cụ thể (`DrawerContentImpl.vue:14-20,305-316`).
  - Đích focus khi đóng: mặc định Trigger. Caller có thể chọn "không" hoặc một phần tử (`DrawerContentImpl.vue:21-27`, `DrawerContent.vue:52-58`).
  - Nội bộ: đang kéo hay không, độ dời, tiến độ vuốt, chiều cao, chiều cao drawer trên cùng, số drawer con đang mở.
- **Bàn phím:**

  | Phím | Kết quả |
  |---|---|
  | Enter, Space trên Trigger | Mở. Đang mở mà focus ở Trigger thì đóng. Điều này chỉ xảy ra ở chế độ tắt, vì hai chế độ kia giữ focus trong content. |
  | Tab, Shift+Tab | Vòng trong content ở mọi chế độ (`DrawerContentImpl.vue:436-443`). |
  | Esc | Đóng, lý do Esc (`DrawerContentImpl.vue:275-278`). |
  | (snap point) | Không có phím nào. Trong `Drawer/` chỉ thấy xử lý Esc (grep `keydown`). |

- **Focus:**
  - Khi mở: như Dialog, cộng đích tuỳ chọn ở trên. Phần tử cụ thể được focus và chọn chữ (`DrawerContentImpl.vue:305-316`).
  - Bẫy focus: chế độ bật và chế độ chỉ-bẫy-focus, trong lúc mở (`DrawerContent.vue:48`).
  - Khi đóng: về Trigger, khác với Dialog (Dialog về phần tử có focus trước đó) (`DrawerContent.vue:52-58,118-124`).
  - Khi drawer unmount như thường lệ, reka trả focus về Trigger cả khi thứ khác đã lấy focus trong lúc animate đóng. Chỉ khi giữ nội dung lúc đóng, reka mới bỏ qua việc trả focus trong trường hợp đó (`DrawerContent.vue:71` so với `79-88,118-124`). Dialog và Popover xét điều này ở cả hai trường hợp.
  - Chế độ chỉ-bẫy-focus và chế độ tắt: không trả focus nếu người dùng đã tương tác bên ngoài (`DrawerContent.vue:79-88`).
- **Pointer: kéo để đóng** (`Drawer/composables/useSwipeDismiss.ts`, gọi tắt `SD`):
  - Chỉ nhận nút chính của chuột (`SD:500-501`).
  - Bắt đầu kéo khi đã dời 1 px (`SD:44,374-376`).
  - Chuột và bút không bắt đầu kéo từ: nút, link, ô nhập, select, textarea, label, `role=button` (`SD:51,505-507`).
  - Màn chạm thì được, trừ thanh trượt range. Chạm mà dời chưa tới 1 px vẫn là click (`SD:577-587`, `DrawerContentImpl.vue:174`, `docs:components/drawer.md:455`).
  - Một vùng có thể tự loại khỏi thao tác kéo với mọi loại input, vd carousel hay bản đồ (`SD:52-54`).
  - Trục bị khoá ngay ở lần di chuyển đầu (`SD:343-357`).
  - Kéo theo hướng đóng: drawer theo tay tuyến tính. Kéo ngược hướng: dời theo căn bậc hai, tức kiểu dây thun (`SD:270-291,409-415`).
  - Khi thả, drawer đóng nếu một trong hai điều kiện đúng (`SD:42,461-468`):
    - đã dời ≥ 40 px;
    - vận tốc theo hướng đóng > 0.3 px/ms.
  - Ngoại lệ: người dùng đã quay đầu. Dời tối đa > 20 px rồi lùi > 10 px thì huỷ đóng (`SD:43,397-407`).
  - Vận tốc tính từ mẫu cuối, dt tối thiểu 16 ms. Lúc thả, nếu mẫu cuối cũ hơn 80 ms thì vận tốc coi là 0 (`SD:49-50,200-209,450-452`).
  - Tiến độ vuốt = độ dời / (kích thước + 40), kẹp trong 0..1 (`SD:418-434`).
  - Trong một vùng cuộn cùng trục: chỉ kéo được khi vùng đó đã ở mép theo hướng đóng, vd đang ở đỉnh và kéo xuống (`SD:81-128,638-664`).
  - Có vùng cuộn ở trục chéo, trên màn chạm (`SD:45-48,298-337`):
    - trục chéo thắng nếu đã vượt 6 px và hơn trục drawer 2 px;
    - trục drawer thắng khi vượt 6 px;
    - chưa bên nào thắng thì để trình duyệt cuộn.
  - Kéo bị kết thúc tại chỗ khi: nhả chuột ngoài cửa sổ, mất capture, hoặc cửa sổ mất focus (`SD:522-537,552-567,696-724`).
  - Trong lúc kéo, bấm ra ngoài hay focus ra ngoài đều bị bỏ qua, không đóng (`DrawerContentImpl.vue:268-303`).
  - Drawer không bắt đầu kéo khi root của nó đang báo "drawer con đang kéo" (`DrawerContentImpl.vue:175`). Xem **Dễ sót**.
  - Đóng bằng vuốt thì giữ nguyên vị trí đang kéo, để animation đóng chạy tiếp từ đó (`SD:470-477`).
  - Vuốt mở (SwipeArea): cùng ngưỡng 40 px và 0.3 px/ms. Ở đây cả màn chạm cũng bỏ qua phần tử tương tác (`DrawerSwipeArea.vue:47-66`, `SD:137,583-585`).
- **Pointer: snap point** (`Drawer/composables/useDrawerSnapPoints.ts`, gọi tắt `SP`):
  - Giá trị hợp lệ (`SP:18-32,44-47`):
    - số 0..1: phần của chiều cao cửa sổ;
    - số > 1: px;
    - chuỗi `px`;
    - chuỗi `rem`: nhân với cỡ chữ gốc, mặc định 16.
  - Đơn vị khác, số âm, NaN bị bỏ. Kết quả được làm tròn.
  - Hai snap cách nhau ≤ 1 px thì bỏ một. Mỗi snap bị kẹp ở min(chiều cao content, chiều cao cửa sổ). Danh sách sắp theo chiều cao (`SP:64-75`).
  - Có snap point thì kéo được cả hai chiều: thu nhỏ và mở rộng (`DrawerContentImpl.vue:149-158`).
  - Khi thả (`SP:7-10,133-141`):
    - đích = vị trí hiện tại + độ dời, kẹp trong [0, chiều cao];
    - nếu |v| ≥ 0.5 px/ms thì cộng thêm v × 300 px, với v kẹp trong ±4.
  - Drawer chỉ đóng khi đích gần trạng thái đóng hơn **mọi** snap point. Lý do đóng là vuốt (`SP:154-160`, `DrawerContentImpl.vue:69-72`).
  - Snap tuần tự: đi một nấc nếu vận tốc cùng chiều kéo và ≥ 0.5, hoặc nếu đích đã vượt qua nấc kề. Không thì giữ nguyên (`SP:162-189`).
  - Snap đang chọn được khớp theo giá trị, hoặc theo chiều cao lệch ±1 px (`SP:83-95`).
  - Luôn đo bằng chiều cao và chiều cao cửa sổ, kể cả drawer trái hay phải (`SP:43,66`, `DrawerContentImpl.vue:107-114`).
- **Lồng nhau:**
  - Drawer con không có overlay riêng (`DrawerOverlay.vue:24,30`).
  - Cha biết số drawer con đang mở, và chiều cao của drawer trên cùng (tức drawer con), để look co lại hay xếp chồng (`DrawerRoot.vue:190-216`, `DrawerContentImpl.vue:117-147`).
  - Con đóng thì cha quay về chiều cao của chính nó, không bao giờ về 0 (`Drawer/Drawer.test.ts:703-738`).
  - Khi có con đang mở, cha ngừng đo lại chiều cao của mình (`DrawerContentImpl.vue:107-111`).
  - Provider biết: có drawer nào đang mở không, tiến độ vuốt, chiều cao drawer trên cùng. Trang nền dùng các giá trị này (`DrawerProvider.vue:53-82`, `DrawerIndent.vue:20-39`).
- **Đóng:**
  - Esc.
  - Bấm ra ngoài, ở cả 3 chế độ. Chế độ bật thì bỏ qua chuột phải và Ctrl+click (`DrawerContent.vue:125-129`).
  - Focus ra ngoài: chế độ bật chặn (`DrawerContent.vue:130`); hai chế độ còn lại đóng.
  - Bấm Trigger: ở hai chế độ không-bật, cú bấm này không tính là bấm ngoài (`DrawerContent.vue:96-98`).
  - Close.
  - Vuốt.
  - Thả về phía đóng khi có snap point.
  - Caller.
  - Caller từ chối được mọi lý do (xem **State**).
- **ARIA:**
  - Trigger: `aria-haspopup=dialog` và `aria-expanded`. `aria-controls` chỉ khi đang mở và content đã có (`DrawerTrigger.vue:37-39`).
  - Content: `role=dialog`, với `aria-labelledby` và `aria-describedby` (`DrawerContentImpl.vue:451-453`).
  - Handle ẩn khỏi AT.
  - Chế độ bật: AT chỉ thấy drawer (`DrawerContent.vue:49-50`).
- **Look cần đọc:**
  - mở/đóng; cạnh;
  - đang kéo (để cắt transition);
  - độ dời x/y; độ dời của snap, có dấu theo cạnh;
  - chiều cao content; chiều cao drawer trên cùng; số drawer con đang mở;
  - tiến độ vuốt 0..1, cho cả overlay lẫn trang nền;
  - hệ số tốc độ thả 0.1..1: thời gian còn lại, kẹp trong 80..360 ms, suy từ vận tốc kẹp trong 0.2..4 px/ms;
  - "đã đóng bằng vuốt", để giữ vị trí;
  - phần bị bàn phím ảo che;
  - có drawer nào mở không, cho trang nền.
  - Nguồn: `Drawer/utils.ts:6-16,57-137`, `DrawerContentImpl.vue:344-361`, `DrawerOverlayImpl.vue:29-37`.
- **RTL:** không. Trái và phải là hướng vật lý; reka không đọc `dir`.
- **Cơ chế DOM không port:**
  - CSS variable và `CSS.registerProperty` → look bám theo ngón tay mỗi frame, giá trị không bị kế thừa.
  - `setPointerCapture`, nghe pointerup trên document và window → kéo ra ngoài content vẫn bám được.
  - `touchmove` với `passive:false`, chỉ `preventDefault` sau khi drawer thắng trục → trả việc cuộn cho trình duyệt đúng lúc trên iOS.
  - `getAnimations()` và rAF → biết khi nào animation xong.
  - ResizeObserver → biết chiều cao.
  - `visualViewport`, padding/scroll chèn thêm, một click tự phát sau cú chạm (`Drawer/composables/useDrawerVirtualKeyboard.ts:9-18,139-158,368,374-377,696-698`) → ô nhập đang có focus luôn nằm trong vùng còn nhìn thấy, cách mép 16 px. Viewport đổi dưới 60 px thì coi là thanh trình duyệt, không phải bàn phím. Tạm ngưng khi người dùng đang pinch-zoom.
  - Khoá cuộn qua overlay, `aria-hidden`, `pointer-events` → như Dialog.
- **Dễ sót:**
  - Ngưỡng 40 px hoặc 0.3 px/ms. Quay đầu > 10 px thì huỷ. Mẫu vận tốc cũ hơn 80 ms thì bỏ (`SD:42-50,397-407,450-468`).
  - Kéo ngược là dây thun, không khoá cứng (`SD:270-291`).
  - Chuột không kéo được từ nút hay link; màn chạm thì được (`SD:505-507,577-587`).
  - Caller từ chối, hoặc drawer controlled không được cập nhật → drawer trượt về (`DrawerContentImpl.vue:176-188`).
  - Snap: chỉ đóng khi gần trạng thái đóng hơn mọi snap. Cú vẩy cộng thêm tối đa 1200 px (`SP:136-160`).
  - Không có phím cho snap point; người dùng bàn phím chỉ có Esc (`DrawerContentImpl.vue:275-278`; grep `keydown` trong `Drawer/` không thấy xử lý phím nào khác). S6 có thể làm tốt hơn, nhưng phải ghi lại.
  - Focus khi đóng về Trigger, khác Dialog (`DrawerContent.vue:52-58`).
  - Khi unmount như thường lệ, reka giành lại focus cho Trigger, kể cả khi một layer khác vừa lấy focus (`DrawerContent.vue:79-88,118-124`). Có vẻ là lỗi. S6 nên làm như Dialog và ghi lại chỗ khác.
  - Docs lệch source ở hai chỗ:
    - lý do `click` và `cancel` có khai báo nhưng không bao giờ được phát (`DrawerRoot.vue:12-19` so với `docs:components/drawer.md:550`);
    - docs nói chế độ chỉ-bẫy-focus "no overlay is rendered" (`docs:components/drawer.md:508-509`), nhưng source vẫn render overlay vì giá trị chế độ đó là truthy (`DrawerOverlay.vue:30`); kéo theo là khoá cuộn (`DrawerOverlayImpl.vue:17-23`).
  - Báo "đang kéo" và tiến độ vuốt lên cha bị lệch một cấp. Con ghi vào chính root của nó, rồi chỉ báo lên ông (`DrawerRoot.vue:217-225`, `DrawerContentImpl.vue:250-259`). Ý định theo comment (`DrawerContentImpl.vue:363-367`): cha đọc tiến độ của con, và không kéo khi con đang kéo. S6 khớp ý định, ghi lại chỗ khác.
  - Cảnh báo dev chỉ khi thiếu Title, không xét Description (`DrawerContentImpl.vue:423-432`).

---

### Popover
`Popover/` · dùng bởi: `popover`.

shadcn căn giữa, cách trigger 4 px (`shadcn:popover/PopoverContent.vue:19-20`). Title và Description của shadcn là `div`/`p` thường, không nối ARIA.

- **Part:**
  - Root: giữ state. Định vị qua Popper (`PopoverRoot.vue:99-105`).
  - Trigger: nút, bấm thì đảo. Là mốc định vị nếu không có Anchor (`PopoverTrigger.vue:31-46`).
  - Anchor: mốc định vị thay cho trigger (`PopoverAnchor.vue:18-32`).
  - Portal.
  - Content: `role=dialog`. Là layer, focus scope và nội dung được định vị (`PopoverContentImpl.vue:56-97`).
  - Arrow: mũi tên 10×5 px (`PopoverArrow.vue:11-15`).
  - Close: nút đóng (`PopoverClose.vue:27`).
- **Trạng thái:**
  - Mở/đóng. Mặc định đóng.
    - Người dùng ghi: Trigger đảo; Close, Esc, bấm ngoài, focus ngoài thì đóng.
    - Caller ghi bất kỳ lúc nào (`PopoverRoot.vue:55-60,74-77`).
  - Modal. Mặc định **tắt**. Bật thì: bẫy focus, AT chỉ thấy popover, bên ngoài trơ, khoá cuộn trang.
  - Giữ nội dung khi đóng. Mặc định không (`PopoverContent.vue:44-53`).
  - Vị trí: phía ưu tiên dưới, căn giữa, khoảng cách 0, lật khi va chạm (`Popper/PopperContent.vue:15-33`). Chi tiết thuộc nhóm Popper.
- **Bàn phím:**

  | Phím | Kết quả |
  |---|---|
  | Enter, Space trên Trigger | Mở. Đang mở mà focus ở Trigger thì đóng, vd khi caller chặn auto-focus lúc mở. |
  | Tab, Shift+Tab | Vòng trong content, kể cả khi non-modal (`PopoverContentImpl.vue:56-58`). |
  | Esc | Đóng. Focus về Trigger. |

- **Focus:**
  - Khi mở: như Dialog. Content không có gì tab được thì focus chính content, và việc đó không bị coi là focus ra ngoài (`DismissableLayer/utils.ts:31-39`, `Popover/Popover.test.ts:198-199`).
  - Bẫy focus: chỉ khi modal, trong lúc mở (`PopoverContentModal.vue:44`).
  - Khi đóng: về Trigger, không về phần tử có focus trước đó (`PopoverContentModal.vue:46-54`, `PopoverContentNonModal.vue:39-51`).
  - Không trả focus khi:
    - thứ khác đã lấy focus;
    - non-modal và người dùng đã tương tác bên ngoài;
    - modal và đóng bằng chuột phải ở ngoài (`PopoverContentModal.vue:55-64`).
- **Pointer:**
  - Trigger phản ứng ở `click`.
  - Modal: bên ngoài trơ, cú bấm ra ngoài đầu tiên chỉ đóng (`PopoverContentModal.vue:45`). Chuột phải ở ngoài **cũng đóng** (Dialog thì không), nhưng không trả focus.
  - Non-modal: bấm Trigger lúc đang mở thì đóng một lần (`PopoverContentNonModal.vue:61-66`).
- **Đóng:**
  - Esc.
  - Bấm ra ngoài.
  - Focus ra ngoài: modal chặn (`PopoverContentModal.vue:65`); non-modal đóng.
  - Close, Trigger, caller.
  - Layer con và layer nhánh không tính là bên ngoài.
  - Caller chặn được từng nguồn đóng, giống Dialog.
- **ARIA:**
  - Trigger: `aria-haspopup=dialog` và `aria-expanded`. `aria-controls` chỉ khi đang mở và content đã có (`PopoverTrigger.vue:40-42`, `Popover/Popover.test.ts:103`).
  - Content: `role=dialog`. `aria-labelledby` trỏ tới Trigger, đúng id mà trigger thực sự render, kể cả id do caller đặt (`PopoverContentImpl.vue:81,92`, `shared/useTriggerId.ts:34-70`).
  - Modal: AT chỉ thấy popover (`PopoverContentModal.vue:24`).
- **Look cần đọc:**
  - mở/đóng;
  - phía và cách căn thực tế sau khi tránh va chạm;
  - transform origin;
  - khoảng còn lại tới biên (rộng, cao);
  - kích thước trigger;
  - vị trí mũi tên, và mũi tên có bị ẩn không (ẩn khi không căn giữa được).
  - Nguồn: `PopoverContentImpl.vue:80-91`, `Popper/PopperContent.vue:447-448`.
- **RTL:** chỉ qua Popper. Căn đầu/cuối (start/end) và transform origin lật theo `dir` (`Popper/PopperContent.vue:200-202,249`, `Popper/Popper.test.ts:46-58`).
- **Cơ chế DOM không port:**
  - Focus guard: span canh ở đầu và cuối body (`PopoverContentImpl.vue:52`) → sự kiện focus quanh nội dung portal nhất quán.
  - Teleport, `aria-hidden`, `pointer-events` → như Dialog.
  - Khoá cuộn khi modal, không cần overlay (`PopoverContentModal.vue:15-19`) → trang nền không cuộn.
  - CSS variable → look đọc được kích thước và transform origin.
- **Dễ sót:**
  - Focus khi đóng về Trigger, khác Dialog (`PopoverContentModal.vue:51`).
  - Modal: chuột phải ở ngoài thì đóng mà không trả focus. Dialog thì không đóng (`PopoverContentModal.vue:55-64` so với `DialogContentModal.vue:72-83`).
  - Modal luôn khoá cuộn dù không có overlay (`PopoverContentModal.vue:15-19`). Dialog khoá theo overlay.
  - Tên truy cập của content là Trigger. Title của shadcn không được nối (`PopoverContentImpl.vue:81`, `shadcn:popover/PopoverTitle.vue`).
  - Anchor tách mốc định vị khỏi Trigger; Trigger vẫn là nút đảo (`PopoverTrigger.vue:31-35`).
  - Non-modal vẫn vòng Tab (`PopoverContentImpl.vue:56-58`).

---

### HoverCard
`HoverCard/` · dùng bởi: `hover-card` (shadcn căn giữa, cách 4 px, `shadcn:hover-card/HoverCardContent.vue:19-20`)

- **Part:**
  - Root: giữ state và các hẹn giờ (`HoverCardRoot.vue:78-125`).
  - Trigger: mặc định là link (`a`), cũng là mốc định vị (`HoverCardTrigger.vue:14-16,54-58`).
  - Portal.
  - Content: nội dung được định vị, là một layer, không có role (`HoverCardContentImpl.vue:87-122`).
  - Arrow: 10×5 px (`HoverCardArrow.vue:11-15`).
- **Trạng thái:**
  - Mở/đóng. Mặc định đóng.
    - Người dùng ghi: hover, focus, Esc, bấm ngoài, cuộn.
    - Caller ghi bất kỳ lúc nào (`HoverCardRoot.vue:47-53,66-69`).
  - Trễ mở 700 ms, trễ đóng 300 ms. Caller đặt (`HoverCardRoot.vue:50-51`).
  - Màn chạm: mặc định bỏ qua. Bật thì một lần chạm đảo trạng thái ngay, không trễ (`HoverCardRoot.vue:52`, `HoverCardTrigger.vue:42-50`).
- **Bàn phím:**

  | Phím | Kết quả |
  |---|---|
  | Tab tới Trigger | Mở sau 700 ms (`HoverCardTrigger.vue:69`). |
  | Tab rời Trigger | Đóng sau 300 ms (`HoverCardTrigger.vue:70`). |
  | Enter | Đi theo link. Đó là hành vi sẵn có của link. |
  | Esc | Đóng ngay (`HoverCardContentImpl.vue:90-93`, `HoverCardRoot.vue:120-125`). |

  Content không vào được bằng Tab (xem **Focus**).
- **Focus:**
  - Không di chuyển focus khi mở hay đóng. Không có focus scope.
  - Các phần tử tab được trong content bị loại khỏi thứ tự Tab (`HoverCardContentImpl.vue:68-69`).
  - Focus ra ngoài không đóng (`HoverCardContentImpl.vue:92`).
- **Pointer:**
  - Con trỏ vào Trigger (trừ chạm) thì hẹn mở 700 ms. Nhiều nguồn cùng lúc (hover rồi focus) dùng chung hạn đầu tiên (`HoverCardRoot.vue:88-99`, `HoverCard/HoverCard.test.ts:278`).
  - Rời Trigger khi chưa mở thì huỷ hẹn mở (`HoverCardTrigger.vue:25-33`, `HoverCardRoot.vue:107-113`).
  - Đã mở, rời Trigger đi về phía content:
    - Không đóng nếu con trỏ còn trong vùng ân hạn: bao lồi của điểm rời (±1 px) và 4 góc của content (`shared/useGraceArea.ts:24-35`).
    - Ra khỏi vùng đó, hoặc đi vào một trigger hover khác, thì hẹn đóng 300 ms (`shared/useGraceArea.ts:54-79`, `HoverCardContentImpl.vue:25-31`).
  - Cờ "đang băng qua" bật khi con trỏ rời trigger hoặc content, và tự tắt 300 ms sau đó, dù con trỏ còn di chuyển. Comment trong source: "Reset the inTransit state if idle/scrolled". Vùng ân hạn thì vẫn còn hiệu lực (`shared/useGraceArea.ts:7-8,24-35`).
  - Vào content thì huỷ hẹn đóng (`HoverCardContent.vue:42`, `HoverCardRoot.vue:89`).
  - Đang nhấn chuột trong content, hoặc đang có chữ được chọn, thì rời ra cũng không đóng. Trong lúc kéo, vùng chọn chữ bị giới hạn trong content (`HoverCardRoot.vue:109`, `HoverCardContentImpl.vue:33-63,111-118`).
- **Đóng:**
  - Rời đi: sau 300 ms.
  - Blur khỏi Trigger: sau 300 ms.
  - Đóng ngay: Esc, bấm ra ngoài, cuộn một tổ tiên của Trigger (`HoverCardContentImpl.vue:72-76`).
  - Chạm lần nữa, nếu đã bật chạm.
  - Caller.
  - Caller chặn được Esc và bấm ra ngoài (reka `escapeKeyDown`, `pointerDownOutside`).
- **ARIA:**
  - Không có gì. Trigger không có `aria-haspopup`, `aria-expanded` hay `aria-controls`. Content không có role (`HoverCardTrigger.vue:59-73`, `HoverCardContentImpl.vue:95-121`).
  - Docs: "Ignored by screen readers", "intended for sighted users only" (`docs:components/hover-card.md:24,295`).
- **Look cần đọc:**
  - mở/đóng;
  - phía và cách căn thực tế;
  - transform origin;
  - khoảng còn lại tới biên;
  - kích thước trigger;
  - cho phép chọn chữ trong content khi đang kéo chọn.
- **RTL:** như Popover (qua Popper).
- **Cơ chế DOM không port:**
  - `user-select` trên body và content → chọn chữ không tràn ra trang.
  - `tabindex=-1` cho các con → content nằm ngoài thứ tự Tab.
  - Nghe scroll ở pha capture → đóng khi tổ tiên cuộn.
  - Đánh dấu trigger hover bằng attribute → nhận ra trigger khác trong vùng ân hạn.
  - `setTimeout(0)` khi rời trigger → kiểm tra sau khi vùng ân hạn đã tính xong.
- **Dễ sót:**
  - Focus bằng bàn phím cũng chịu trễ 700/300 ms. Docs chỉ viết "Tab: Opens/closes" (`docs:components/hover-card.md:301-302`).
  - Hạn mở không bị đặt lại khi focus đến sau hover (`HoverCardRoot.vue:91-93`).
  - Có chữ được chọn trong content thì không tự đóng khi rời ra (`HoverCardRoot.vue:109`).
  - Content không vào được bằng bàn phím (`HoverCardContentImpl.vue:68-69`).
  - Mặc định bỏ qua màn chạm (`HoverCardTrigger.vue:43`, `HoverCard/HoverCard.test.ts:36`).
  - Cuộn một tổ tiên của trigger thì đóng ngay (`HoverCardContentImpl.vue:72-76`).

---

### Tooltip và TooltipProvider
`Tooltip/` · dùng bởi:

- `tooltip`: Provider của shadcn mặc định trễ 0 ms (`shadcn:tooltip/TooltipProvider.vue:6`). `Tooltip` của shadcn không tự bọc Provider.
- `sidebar`: `SidebarProvider` bọc một Provider trễ 0 ms (`shadcn:sidebar/SidebarProvider.vue:69`). `SidebarMenuButton` có tooltip bên phải, căn giữa, chỉ hiện khi sidebar thu gọn và không phải mobile (`shadcn:sidebar/SidebarMenuButton.vue:37-41`).

Phần dưới:

- **Part:**
  - Provider: chia sẻ độ trễ và cửa sổ bỏ trễ giữa các tooltip. Bắt buộc: Root không có Provider thì báo lỗi (`TooltipRoot.vue:100`, `shared/createContext.ts:33-46`).
  - Root: giữ state và hẹn giờ.
  - Trigger: nút (`type=button` khi là button). Là mốc định vị (`TooltipTrigger.vue:22-24,113-131`).
  - Portal.
  - Content: nội dung được định vị. Mặc định ở trên, căn giữa, cách 0 (`TooltipContentImpl.vue:75-89`). Kèm một phần tử ẩn khỏi mắt, `role=tooltip`, mang chữ để AT đọc (`TooltipContentImpl.vue:131-137`).
  - Arrow: 10×5 px (`TooltipArrow.vue:25-29`).
- **Trạng thái:**
  - Mở/đóng. Mặc định đóng.
    - Người dùng ghi: hover, focus, blur, bấm, Esc, bấm ngoài.
    - Caller ghi bất kỳ lúc nào (`TooltipRoot.vue:80-88,109-112`).
  - Kiểu mở: đóng, mở sau trễ, hoặc mở ngay (`TooltipRoot.vue:131-135`).
  - Tuỳ chọn hành vi. Root ghi đè được; không ghi đè thì lấy từ Provider (`TooltipRoot.vue:102-107`):
    - Trễ mở: 700 ms (reka `delayDuration`, `TooltipProvider.vue:72`).
    - Cửa sổ bỏ trễ: 300 ms. Khi đang có một tooltip mở, hoặc trong 300 ms sau khi nó đóng, con trỏ tới một trigger khác thì tooltip đó mở ngay, không trễ (reka `skipDelayDuration`, `TooltipProvider.vue:73,80-97`, `TooltipRoot.vue:163-167`).
    - Nội dung hover được: mặc định có. Con trỏ đi vào content thì không đóng. Tắt thì đóng ngay khi rời Trigger (reka `disableHoverableContent`, `TooltipContent.vue:41`, `TooltipRoot.vue:168-176`).
    - Bấm Trigger thì đóng: mặc định có. Tắt thì nhấn và click không đóng, và nhấn trigger không tính là bấm ngoài (reka `disableClosingTrigger`, `TooltipTrigger.vue:68-74,106-109`, `TooltipContentImpl.vue:108-113`).
    - Chỉ mở khi focus đến từ bàn phím: mặc định tắt. Bật thì focus không khớp `:focus-visible` không mở tooltip; docs nêu ví dụ: khi đổi tab trình duyệt, khi một dialog đóng (reka `ignoreNonKeyboardFocus`, `TooltipProvider.vue:48-55`, `TooltipTrigger.vue:96-97`).
    - Vô hiệu: mặc định tắt. Bật thì Trigger không phản ứng gì (`TooltipTrigger.vue:36-38`).
  - Vị trí: tuỳ chọn của chính Content thắng mặc định chung đặt ở Provider; Provider thắng mặc định sẵn có (`TooltipContentImpl.vue:75-89`).
- **Bàn phím:** docs khớp source (`docs:components/tooltip.md:367-385`).

  | Phím | Kết quả |
  |---|---|
  | Tab tới Trigger | Mở ngay, không trễ (`TooltipTrigger.vue:92-100`, `TooltipRoot.vue:142-146`). |
  | Tab rời Trigger | Đóng (`TooltipTrigger.vue:102-104`). |
  | Enter, Space | Đóng, vì nút được kích hoạt sinh ra click (`TooltipTrigger.vue:106-109`). |
  | Esc | Đóng. Tooltip là layer trên cùng (`TooltipContentImpl.vue:104-116`). |

- **Focus:** không bao giờ di chuyển focus. Content không nhận focus.
- **Pointer:**
  - Mở ở lần di chuột đầu tiên trên Trigger (pointermove), không phải lúc con trỏ đi vào. Màn chạm bị bỏ qua (`TooltipTrigger.vue:76-85`).
  - Đã mở một lần trong lượt hover thì không mở lại, cho tới khi con trỏ rời ra rồi vào lại (`TooltipTrigger.vue:83,87-90`).
  - Một tooltip khác đang có con trỏ "băng qua" về phía nó thì tooltip này không mở. Việc chặn kéo dài tối đa 300 ms (`TooltipTrigger.vue:80`, `TooltipContentHoverable.vue:18`, `shared/useGraceArea.ts:8`).
  - Rời Trigger trước khi tooltip mở thì huỷ hẹn (`TooltipRoot.vue:172-175`).
  - Vùng ân hạn khi nội dung hover được: giống HoverCard. Ra khỏi vùng thì đóng **ngay**, không có trễ đóng (`TooltipContentHoverable.vue:16-21`, `shared/useGraceArea.ts:24-35,54-79`).
  - Content đè lên một trigger tooltip khác ngay dưới con trỏ thì đóng ngay, để trigger kia hover được (`TooltipContentHoverable.vue:23-43`).
  - Nhấn (pointerdown) Trigger thì đóng. Focus do chính cú nhấn đó không mở lại tooltip; cờ "đang nhấn" chỉ tắt 1 ms sau khi nhả (`TooltipTrigger.vue:54-74,92-94`).
- **Đóng:**
  - Rời đi: theo vùng ân hạn, hoặc ngay lập tức nếu nội dung không hover được.
  - Blur.
  - Nhấn hoặc click Trigger.
  - Esc.
  - Bấm ra ngoài.
  - Một tooltip khác mở: mỗi lúc chỉ một tooltip mở (`TooltipRoot.vue:117-122`, `TooltipContentImpl.vue:98-99`).
  - Một tổ tiên của Trigger cuộn (`TooltipContentImpl.vue:91-97`).
  - Content đè lên trigger khác.
  - Caller.
  - Focus ra ngoài không đóng (`TooltipContentImpl.vue:114`).
  - Caller chặn được Esc và bấm ra ngoài (reka `escapeKeyDown`, `pointerDownOutside`).
- **ARIA:**
  - Trigger: `aria-describedby` trỏ tới phần tử ẩn `role=tooltip`, chỉ khi đang mở và phần tử đó đã có (`TooltipTrigger.vue:120-122`, `TooltipContentImpl.vue:69-73`).
  - Chữ AT đọc: nhãn ghi đè của caller, không có thì toàn bộ chữ của content (`TooltipContentImpl.vue:67`).
  - Không có `aria-expanded`, không có `aria-haspopup`.
- **Look cần đọc:**
  - kiểu mở (đóng, mở sau trễ, mở ngay), cho cả trigger và content;
  - phía và cách căn thực tế;
  - transform origin;
  - khoảng còn lại tới biên;
  - kích thước trigger;
  - vị trí mũi tên.
- **RTL:** qua Popper, như Popover. Không chỉ định hướng thì lấy từ cấu hình chung, mặc định `ltr` (`shared/useDirection.ts:11-16`, `Popper/PopperContent.vue:249`).
- **Cơ chế DOM không port:**
  - Sự kiện chung trên document (`TooltipRoot.vue:121`) → mỗi lúc chỉ một tooltip mở.
  - `elementsFromPoint` → biết content đang đè trigger khác.
  - Phần tử VisuallyHidden mang `role=tooltip` → AT đọc chữ mà không phụ thuộc phần nhìn thấy.
  - `:focus-visible` → phân biệt focus từ bàn phím.
  - Nghe pointerup một lần trên document, cộng `setTimeout(1)` → bỏ qua focus do cú nhấn.
  - Nghe scroll ở pha capture → đóng khi tổ tiên cuộn.
  - `type=button` → trigger không submit form.
- **Dễ sót:**
  - Bắt buộc có Provider (`TooltipRoot.vue:100`).
  - Mở theo pointermove. Đóng bằng click rồi thì không mở lại cho tới khi rời ra và vào lại (`TooltipTrigger.vue:76-90`).
  - Bàn phím mở không trễ. Enter và Space đóng (`TooltipTrigger.vue:92-109`).
  - Cửa sổ bỏ trễ 300 ms. Lúc đầu Provider ở trạng thái "có trễ" (`TooltipProvider.vue:80`).
  - Mỗi lúc chỉ một tooltip mở (`TooltipRoot.vue:117-122`, `TooltipContentImpl.vue:98-99`).
  - Content đè lên trigger khác thì đóng (`TooltipContentHoverable.vue:23-43`).
  - Esc đóng tooltip trước, chưa đóng dialog bên dưới, vì tooltip là layer trên cùng (`DismissableLayer/DismissableLayer.vue:153-155`).
  - Docs bày cách bọc nút bị vô hiệu trong `span` để vẫn hiện tooltip (`docs:components/tooltip.md:185`). Đó là giới hạn của DOM: nút disabled không phát sự kiện. S6 không bị ràng buộc bởi điều này.

---

### Toast
`Toast/` · dùng bởi: không component nào.

Mục này viết ngắn. `sonner` của shadcn dùng `vue-sonner` (`shadcn:sonner/Sonner.vue:2-5`). Toast của reka là tham chiếu gần nhất.

- **Part:**
  - Provider: các mặc định chung cho mọi toast.
  - Viewport: vùng `region` có nhãn, chứa danh sách toast.
  - Root: một toast, là một mục trong danh sách và tab được (`ToastRootImpl.vue:254-257`).
  - Title, Description.
  - Action: bắt buộc có chữ thay thế cho AT, thiếu thì báo lỗi (`ToastAction.vue:44-45`).
  - Close.
  - Positioner: neo toast vào một phần tử, mặc định ở phía trên (`ToastPositioner.vue:40-41`).
  - Manager: thêm, cập nhật, đóng toast từ bất cứ đâu. Với một promise: toast "đang tải" rồi chuyển thành thành công hoặc lỗi (`createToastManager.ts:86-101`).
- **Trạng thái:**
  - Mỗi toast:
    - mở/đóng; mặc định mở khi tạo (`ToastRoot.vue:29-34`);
    - thời gian hiển thị: mặc định 5000 ms; ≤ 0 hoặc vô hạn thì không tự đóng; trạng thái "đang tải" coi là vô hạn (`ToastProvider.vue:121`, `ToastRootImpl.vue:97-99`, `ToastRoot.vue:59-61`);
    - thời gian còn lại, cập nhật tối đa 60 lần/giây (`ToastRootImpl.vue:92-95`);
    - mức nhạy: foreground (mặc định) hoặc background;
    - status.
  - Ai ghi:
    - timer: đóng khi hết giờ;
    - người dùng: Close, Action, Esc, vuốt;
    - caller hoặc manager: mọi trường.
  - Thêm một toast có id đã tồn tại thì cập nhật tại chỗ và chạy lại timer (`createToastManager.ts:65-71`, `ToastRootImpl.vue:187-194`).
  - Provider (`ToastProvider.vue:120-125,179-182,94-99`):
    - nhãn cho mỗi toast "Notification"; bắt buộc không rỗng;
    - hướng vuốt: phải;
    - ngưỡng vuốt: 50 px;
    - tạm dừng khi hover: bật;
    - tạm dừng khi cửa sổ mất focus: bật;
    - giới hạn số toast: không có.
  - Viewport: phím tắt F8; nhãn "Notifications ({hotkey})" (`ToastViewport.vue:37-41`).
- **Bàn phím:**

  | Phím | Kết quả |
  |---|---|
  | F8 | Focus danh sách toast. Đó là phím mặc định; với tổ hợp phím thì mọi phím đều phải khớp (`ToastViewport.vue:72-77`). |
  | Tab | Toast mới nhất trước, rồi các control của nó, rồi tới toast cũ hơn. Tới mép thì Tab ra phần còn lại của trang (`ToastViewport.vue:247-280,313-324`). |
  | Shift+Tab | Ngược lại. Từ chính danh sách thì ra phần tử đứng trước nó (`ToastViewport.vue:259-264`). |
  | Tab từ ngoài vào | Đi xuôi: vào toast mới nhất. Đi ngược: vào control cuối của toast cũ nhất (`ToastViewport.vue:339-352,370-383`). |
  | Esc, khi focus trong một toast | Đóng toast đó. Esc không lan ra dialog hay popover bao quanh (`ToastRootImpl.vue:196-209`). |
  | Enter, Space trên Action hoặc Close | Đóng. Action có thể không đóng nếu caller chọn vậy (`ToastAction.vue:17-19,47-52`). |

- **Focus:**
  - Toast không tự lấy focus khi hiện.
  - Đóng bằng bàn phím trên Close hoặc Action khi focus đang trong toast: focus chuyển về danh sách (`ToastRootImpl.vue:110-121`).
  - Đóng bằng Esc thì focus **không** chuyển, vì lệnh đóng không kèm sự kiện (`ToastRootImpl.vue:207`). Có vẻ là thiếu sót. S6 nên giữ focus lại.
- **Pointer (vuốt)** (`ToastRootImpl.vue`):
  - Chỉ nút chính (`:272`).
  - Bắt đầu khi dời theo hướng vuốt quá 2 px (chuột) hoặc 10 px (chạm). Dời sai hướng quá ngưỡng đó thì bỏ vuốt cho cả lần nhấn này (`:289,296-305`).
  - Độ dời bị kẹp về phía hướng vuốt; không kéo ngược được (`:284-288`).
  - Khi nhả: dời > 50 px thì đóng, trừ khi caller chặn; không thì trượt về (`:307-327`, `ToastRoot.vue:127-138`).
  - Không xét vận tốc.
  - Click ngay sau một lần vuốt bị nuốt (`:328-332`).
  - Nhấn giữ một toast cũng giữ tạm dừng khi tạm dừng bằng hover bị tắt (`ToastViewport.vue:205-218`).
- **Tạm dừng:**
  - Mọi toast dừng và chạy cùng nhau (`ToastViewport.vue:131-143`, `ToastRootImpl.vue:160-182`).
  - Dừng khi (`ToastViewport.vue:167-175`):
    - focus nằm trong viewport (luôn luôn);
    - đang nhấn một toast;
    - hover, trừ khi tắt;
    - cửa sổ mất focus, trừ khi tắt.
  - Chạy tiếp từ thời gian còn lại (`ToastRootImpl.vue:163-174`).
  - Toast mở ra trong lúc đang dừng thì chờ tới khi chạy lại (`ToastRootImpl.vue:192-193`).
- **Đóng:**
  - Hết giờ.
  - Close, Action.
  - Esc khi focus trong toast.
  - Vuốt.
  - Caller hoặc manager: đóng một toast hoặc tất cả.
  - Bấm ra ngoài không đóng toast.
- **ARIA:**
  - Viewport: `region` với nhãn "Notifications (F8)" (`ToastViewport.vue:328-332`). Bên trong là danh sách (`ol`); mỗi toast là một mục (`li`) (`ToastViewport.vue:40`, `ToastRoot.vue:33`).
  - Thông báo qua một vùng live ẩn: foreground đọc ngay (assertive), background đọc lịch sự (polite); reka đặt `role=alert` (`ToastRootImpl.vue:226-231`).
  - Nội dung được đọc (`Toast/utils.ts:52-78`, `ToastClose.vue:22`, `ToastAction.vue:56-60`):
    - "Notification" rồi tới chữ của toast;
    - đọc từng khúc để có ngắt nghỉ;
    - bỏ phần bị ẩn;
    - bỏ Close;
    - Action được thay bằng chữ thay thế của nó.
  - Chữ trong vùng live xuất hiện sau 2 frame (để NVDA đọc), hoặc sau 1000 ms nếu chưa có 2 frame, tuỳ cái nào tới trước. Hẹn giờ 1000 ms không gỡ nó: nó còn cho tới khi toast unmount, hoặc được tạo lại khi toast được cập nhật (`ToastAnnounce.vue:9-33`, `ToastRootImpl.vue:226-228`).
  - Toast được cập nhật thì đọc lại (`ToastRootImpl.vue:125-130,228`).
  - Toast vượt giới hạn: trơ, không focus được, không tương tác được (`ToastRootImpl.vue:264-265`).
  - Viewport là layer "nhánh": tương tác với toast không làm đóng dialog hay popover đang mở (`ToastViewport.vue:328`).
- **Look cần đọc:**
  - mở/đóng;
  - pha vuốt (bắt đầu, đang di, huỷ, kết thúc) và độ dời; hướng vuốt;
  - vị trí trong chồng, mới nhất là 0;
  - độ lệch: tổng chiều cao các toast mới hơn;
  - chiều cao tự nhiên của toast; chiều cao toast trên cùng;
  - chồng có đang bung ra không: bung khi hover hoặc có focus. Sau khi con trỏ rời, chồng chỉ thu lại khi các toast đang đóng đã animate xong (`ToastViewport.vue:93-121`, `ToastProvider.vue:153-174,208-210`);
  - có vượt giới hạn không;
  - status;
  - thời gian còn lại và tổng thời gian.
- **RTL:** không. Hướng vuốt là vật lý.
- **Cơ chế DOM không port:**
  - FocusProxy (span canh) → Tab vào và ra danh sách đúng thứ tự dù toast được portal.
  - `inert` → toast vượt giới hạn trơ.
  - `touch-action:none` và `user-select:none` → vuốt không biến thành cuộn hay bôi chữ.
  - Double rAF, dự phòng bằng hẹn giờ 1 s → NVDA đọc được thông báo.
  - Teleport vào viewport → toast nằm trong danh sách.
  - CustomEvent tạm dừng/chạy lại → mọi toast dừng cùng nhau.
- **Dễ sót:**
  - Tab bắt đầu từ toast mới nhất, không theo thứ tự DOM (`ToastViewport.vue:313-324`).
  - Esc chỉ đóng toast đang chứa focus, và dừng ở đó. Esc trong một layer mở bên trong toast thuộc về layer đó (`ToastRootImpl.vue:196-209`).
  - Focus trong danh sách luôn tạm dừng, kể cả khi đã tắt tạm dừng bằng hover và bằng mất focus cửa sổ (`ToastViewport.vue:167-175`).
  - Toast mở ra trong lúc đang dừng chưa đếm giờ (`ToastRootImpl.vue:192-193`).
  - Toast vượt giới hạn không bị gỡ. Nó còn đó, nhưng trơ (`ToastProvider.vue:94-99`, `ToastRootImpl.vue:264-265`).
  - Vùng thông báo phải tới được AT. Nằm trong cây bị ẩn khỏi AT thì không được đọc (`Toast/Toast.test.ts:154-167`).
  - Action thiếu chữ thay thế là lỗi, không chỉ là cảnh báo (`ToastAction.vue:44-45`).

---

### Ghi chú chung cho nhóm 3
- **Esc:** chỉ layer trên cùng nhận (`DismissableLayer/DismissableLayer.vue:153-155`). Layer đang ẩn nhưng giữ nội dung thì không nằm trong stack. Tooltip đang mở trong dialog được đóng trước. Riêng Esc trong toast thì không lan ra ngoài.
- **IME:** reka không xét trạng thái soạn chữ (composition) khi nhận Esc (`DismissableLayer/DismissableLayer.vue:146-163`). S6 tự quyết, và ghi lại.
- **Bấm ra ngoài:**
  - Chuột và bút: tính ở pointerdown. Màn chạm: tính ở click.
  - Cú bấm mở một layer không tự đóng nó.
  - Tương tác trong layer con hoặc layer nhánh không phải "bên ngoài".
  - Modal: bên ngoài trơ, cú bấm đầu tiên chỉ đóng.
  - Nguồn: `DismissableLayer/utils.ts:27-68,130-176`.
- **Đích focus khi đóng, từng primitive:**
  - Dialog, AlertDialog: phần tử có focus lúc mở; dự phòng là Trigger.
  - Popover, Drawer: Trigger.
  - HoverCard, Tooltip: không di chuyển focus.
  - Toast: về danh sách, chỉ khi đóng bằng bàn phím trên Close hoặc Action.
  - Dialog, AlertDialog, Popover: không giành lại focus mà thứ khác đã lấy. Drawer của reka chỉ làm vậy khi giữ nội dung lúc đóng (`DrawerContent.vue:71` so với `118-124`). S6 nên làm vậy ở mọi primitive.
- **Auto-focus khi mở:**
  - Theo thứ tự tab, tabindex dương đi trước.
  - Ô nhập thì chọn hết chữ.
  - Không cuộn trang.
  - Caller chặn được, trừ ở AlertDialog của reka.
- **Tab vòng trong content cả khi non-modal:** Dialog, Popover, Drawer.
- **Modal gồm 4 bảo đảm:** bẫy focus; AT chỉ thấy layer; bên ngoài trơ với pointer; khoá cuộn. Khoá cuộn khác nhau:
  - Dialog: theo overlay;
  - Popover modal: khoá suốt lúc hiện, không cần overlay;
  - Drawer: theo overlay, trong lúc mở.
- **Thời gian của nhóm hover:**
  - HoverCard: mở sau 700 ms, đóng sau 300 ms.
  - Tooltip: mở sau 700 ms (shadcn dùng 0), cửa sổ bỏ trễ 300 ms, đóng ngay khi ra khỏi vùng ân hạn.
  - Cờ "đang băng qua" tự tắt 300 ms sau khi con trỏ rời đi.
  - Vùng ân hạn: bao lồi của điểm rời (±1 px) và hình chữ nhật đích.
- **Animation đóng:** nội dung còn đó trong lúc animate đóng, và state đọc là "đóng". Mở lại giữa chừng thì giữ nguyên nội dung và bản ghi focus.
- **Tham chiếu ARIA** (`aria-controls`, `aria-describedby` từ trigger) chỉ trỏ vào phần tử đang có thật (`shared/useTriggerId.ts:208-218`).
- **Docs lệch source, source thắng:**
  - Dialog, AlertDialog: đích focus khi đóng.
  - Drawer: lý do `click` và `cancel`; overlay ở chế độ chỉ-bẫy-focus.
  - HoverCard: độ trễ khi focus bằng bàn phím.

## 8. Nhóm 4: menu

### Menu
`Menu/` · dùng bởi: không component nào trực tiếp. Là engine của `DropdownMenu`, `ContextMenu`, `Menubar` (shadcn-vue `dropdown-menu`, `context-menu`, `menubar`).

- **Part:**
  - Root: không vẽ gì. Giữ mở/đóng, dir, modal, giữ-mounted, cờ "input cuối là phím" (`Menu/MenuRoot.vue:80-116`).
  - Content: `role=menu`, `aria-orientation=vertical`. Container nhận được focus nhưng không phải tab stop. Là lớp dismiss và popup định vị (`Menu/MenuContentImpl.vue:395-461`; `RovingFocus/RovingFocusGroup.vue:152`).
  - Item: `role=menuitem`. Không bao giờ là tab stop (`Menu/MenuItemImpl.vue:79-80`).
  - CheckboxItem: `role=menuitemcheckbox` (`Menu/MenuCheckboxItem.vue:49-53`).
  - RadioGroup: `role=group`, giữ giá trị đang chọn. RadioItem: `role=menuitemradio` (`Menu/MenuRadioGroup.vue:55-58`; `Menu/MenuRadioItem.vue:47-51`).
  - ItemIndicator: chỉ tồn tại khi item cha checked hoặc indeterminate. Caller có thể ép luôn tồn tại (`Menu/MenuItemIndicator.vue:42-48`).
  - Group: `role=group`. Tên lấy từ Label nằm trong nó (`Menu/MenuGroup.vue:20-31`).
  - Label: chữ thường. Không focus. Phím và typeahead bỏ qua (`Menu/MenuLabel.vue:15-24`).
  - Separator: `role=separator`, ngang. Không focus (`Menu/MenuSeparator.vue:13-20`).
  - Arrow: mũi tên trang trí trỏ về anchor. Ba wrapper mặc định 10×5 (`DropdownMenu/DropdownMenuArrow.vue:11-15`; `ContextMenu/ContextMenuArrow.vue:12-13`; `Menubar/MenubarArrow.vue:12-13`).
  - Anchor: mốc định vị. Trigger của mọi wrapper và SubTrigger dùng nó (`Menu/MenuAnchor.vue:11-16`; `DropdownMenu/DropdownMenuTrigger.vue:36`; `ContextMenu/ContextMenuTrigger.vue:132-135`; `Menubar/MenubarTrigger.vue:50`; `Menu/MenuSubTrigger.vue:145`).
  - Sub: trạng thái một submenu. SubTrigger: một item mở submenu. SubContent: một `role=menu` thứ hai (`Menu/MenuSub.vue:37-86`; `Menu/MenuSubTrigger.vue:145-179`; `Menu/MenuSubContent.vue:52-116`).
- **Trạng thái:**
  - Mở/đóng của root. Người dùng ghi: trigger, chọn item, `Escape`, bấm hoặc focus ra ngoài. Caller ghi khi điều khiển. `true`: hiện content, focus vào. `false`: đóng cả cây, mọi submenu (`Menu/MenuRoot.vue:89-116`; `Menu/MenuSub.vue:52-57`). Engine chỉ có dạng điều khiển. Wrapper thêm dạng tự giữ với giá trị đầu.
  - Mở/đóng của mỗi submenu. Người dùng ghi: di chuột 100 ms, click, phím mở/đóng, focus rời, highlight sang item khác. Caller ghi được. Bị ép `false` mỗi khi trạng thái của cha đổi (`Menu/MenuSub.vue:37-57`).
  - Item đang highlight, mỗi content một cái. Nội bộ: chuột, phím, typeahead, filter ghi. Caller không ghi. Xoá khi chuột rời item, khi content ẩn (`Menu/MenuItemImpl.vue:36-72`; `Menu/MenuContentImpl.vue:159-175`).
  - Bộ đệm typeahead, mỗi content một cái. Tự xoá 1000 ms sau phím cuối (`shared/useTypeahead.ts:15-16`).
  - Checked của CheckboxItem: `true` / `false` / indeterminate. **Chỉ caller giữ.** Người dùng chọn = yêu cầu đổi: indeterminate → `true`, còn lại → đảo. Caller không lưu thì item không đổi (`Menu/MenuCheckboxItem.vue:26-28,43,54-64`).
  - Giá trị của RadioGroup. **Chỉ caller giữ.** Chọn RadioItem = yêu cầu giá trị = giá trị item, kể cả khi item đã được chọn (không bỏ chọn). Item checked khi bằng `===` (`Menu/MenuRadioGroup.vue:30-52`; `Menu/MenuRadioItem.vue:39-57`).
  - Cờ "input cuối là phím": toàn cục. `keydown` → có. `pointerdown` hoặc `pointermove` → không (`shared/useIsUsingKeyboard.ts:10-16`).
  - Chỉ caller cấu hình: modal (mặc định bật), dir (kế thừa, mặc định LTR), giữ content khi đóng (mặc định không), loop phím (mặc định không), item disabled, chữ typeahead riêng của item, thời gian grace submenu 300 ms (`Menu/MenuRoot.vue:80-87`; `Menu/MenuContentImpl.vue:83-88`; `Menu/MenuItemImpl.vue:4-12`; `Menu/MenuSubTrigger.vue:5-13,28-30`; `shared/useDirection.ts:11-15`).
  - Caller được huỷ: dời focus khi mở, trả focus khi đóng, đóng do `Escape`, đóng do bấm hoặc focus ngoài, đóng sau khi chọn (`Menu/MenuContentImpl.vue:64-71,253-256,403-412`; `Menu/MenuItem.vue:5-11`).
- **Bàn phím:** content và item (`Menu/MenuContentImpl.vue:265-314`; `Menu/MenuItem.vue:69-84`; `Menu/utils.ts:8-11`):

| Phím | Kết quả |
|---|---|
| `ArrowDown` | item bật kế tiếp của chính content này. Từ container → item đầu. Ở cuối thì dừng, trừ khi loop |
| `ArrowUp` | item bật trước. Từ container → item cuối, kể cả khi không loop |
| `Home` / `End` | item bật đầu / cuối |
| `PageUp` / `PageDown` | đầu / cuối, **chỉ** khi container giữ focus. Trên item: không làm gì |
| `Enter` / `Space` trên item | chọn item. `Space` không chọn khi filter đang có chữ |
| ký tự in được | typeahead. Không khi giữ Ctrl/Alt/Meta, không trong ô nhập, không bao giờ với `Space` |
| `Tab` | modal: bị chặn. Non-modal: focus đi ra, menu đóng |
| `Escape` | đóng cả cây |

SubTrigger (`Menu/MenuSubTrigger.vue:127-141`; `Menu/utils.ts:12-15`):

| Phím | Kết quả |
|---|---|
| `Enter`, `Space`, `ArrowRight` (LTR) / `ArrowLeft` (RTL) | mở submenu. Focus vào submenu → item đầu. `Space` bị bỏ khi filter có chữ (`Menu/MenuSubTrigger.vue:128-130`) |

SubContent (`Menu/MenuSubContent.vue:87-112`; `Menu/utils.ts:16-19`):

| Phím | Kết quả |
|---|---|
| `ArrowLeft` (LTR) / `ArrowRight` (RTL) | đóng submenu này. Focus về SubTrigger |
| `Escape` | đóng **cả cây** |

  - CheckboxItem, RadioItem: như item. Chọn = đổi checked, rồi đóng cả cây nếu không bị huỷ (`Menu/MenuCheckboxItem.vue:54-64`; `Menu/MenuRadioItem.vue:52-57`).
  - Typeahead: so tiền tố, không phân biệt hoa thường. Chữ = chữ typeahead riêng; rỗng thì chữ hiển thị đã trim. Gõ lặp một ký tự = xoay vòng các item bắt đầu bằng nó. Tìm từ item hiện tại trở đi; chuỗi một ký tự bỏ qua item hiện tại. Bỏ item disabled. Mỗi content (mỗi submenu) có bộ đệm riêng (`shared/useTypeahead.ts:9-12,78-96`; `Collection/Collection.ts:39-54`; `Menu/MenuContentImpl.vue:295-302`).
- **Focus:**
  - Mở: focus vào container, không cuộn trang. Input cuối là phím → item đang highlight, không có thì item bật đầu, nhận focus. Là pointer → không item nào (`Menu/MenuContentImpl.vue:253-263,420-424`; `RovingFocus/RovingFocusGroup.vue:102-113`).
  - Không có roving tabindex trong content. Phím dời focus thẳng giữa các item (`Menu/MenuContentImpl.vue:276-289`).
  - Modal: focus bị nhốt khi mở (`Menu/MenuRootContentModal.vue:30`). Non-modal và submenu: không nhốt (`Menu/MenuRootContentNonModal.vue:23`; `Menu/MenuSubContent.vue:69`).
  - Chuột = focus. Item dưới chuột nhận focus. Chuột rời item → focus về container, không item nào highlight. Nếu một ô nhập trong menu giữ focus thì chỉ highlight, focus ở lại ô (`Menu/MenuItemImpl.vue:38-72`; `Menu/MenuContentImpl.vue:360-370`).
  - Hệ quả: sau khi chuột rời, phím bắt đầu lại từ đầu hoặc cuối (`shared/useArrowNavigation.ts:177-181`).
  - Submenu mở bằng phím → focus vào submenu → item đầu. Mở bằng chuột → focus ở lại SubTrigger (`Menu/MenuSubTrigger.vue:131-140`; `Menu/MenuSubContent.vue:70-73`).
  - Submenu đóng bằng phím đóng → focus về SubTrigger. Cha có filter → focus về filter, SubTrigger được highlight (`Menu/MenuSubContent.vue:94-112`). Submenu đóng cách khác → không trả focus (`Menu/MenuSubContent.vue:74`).
  - Root đóng: focus đi đâu do wrapper quyết (xem từng wrapper).
  - Content giữ-mounted bị ẩn: lần mở sau như mới, không highlight, typeahead rỗng (`Menu/MenuContentImpl.vue:165-174`).
- **Pointer:**
  - Hover và highlight chỉ với chuột. Chạm và bút chỉ có tap = click (`Menu/MenuItemImpl.vue:39`; `Menu/MenuSubTrigger.vue:67,83`; `Menu/utils.ts:97-99`).
  - Chuột lên item disabled → bỏ highlight, focus về container (`Menu/MenuItemImpl.vue:41-43`).
  - Nhấn ở chỗ khác, nhả trên một item → chọn item đó (kéo-thả-chọn) (`Menu/MenuItem.vue:54-68`).
  - SubTrigger mở submenu 100 ms sau lần **di** chuột đầu trên nó (không phải lúc vào). Rời trước 100 ms → huỷ (`Menu/MenuSubTrigger.vue:66-85`).
  - Click SubTrigger → mở ngay. Click khi đang mở → không đóng (`Menu/MenuSubTrigger.vue:161-171`).
  - Vùng grace. Chuột rời SubTrigger khi submenu đang hiện → dựng đa giác 5 điểm. Điểm đầu là điểm rời, lùi 5 px về phía **ngược** submenu, để chính điểm rời nằm trong đa giác. Bốn điểm còn lại là 4 góc của submenu: gần-trên, xa-trên, xa-dưới, gần-dưới. Phía của submenu lấy từ cạnh thực tế sau flip (`Menu/MenuSubTrigger.vue:87-109`).
  - Trong thời gian grace (300 ms), nếu chuột nằm trong đa giác **và** đang đi về phía submenu (theo chiều x của lần di gần nhất): item khác không lấy highlight, rời item không xoá highlight, submenu không đóng (`Menu/MenuSubTrigger.vue:111-115`; `Menu/MenuContentImpl.vue:243-251,325-341,352-377`).
  - Submenu đóng khi highlight sang một item **khác** của cha. Highlight biến mất (chuột rời mọi item) → submenu vẫn mở (`Menu/MenuContentImpl.vue:218-229`).
  - Chuột vào một content có filter → filter nhận focus (`Menu/MenuContentImpl.vue:343-350`).
- **Đóng:**
  - Chọn item → đóng cả cây. Chọn là sự kiện huỷ được; bị huỷ thì menu ở lại (`Menu/MenuItem.vue:32-46`). Không có cờ "close on select". Checkbox và radio cũng đóng mặc định.
  - `Escape` ở tầng nào cũng đóng cả cây. Chỉ lớp trên cùng nhận `Escape` (`Menu/MenuSubContent.vue:87-93`; `DismissableLayer/DismissableLayer.vue:146-158`).
  - Bấm ra ngoài cả cây → đóng (`Menu/MenuRootContentModal.vue:33`; `Menu/MenuRootContentNonModal.vue:26`). Bấm trong submenu không đóng cha (`DismissableLayer/utils.ts:58-67`).
  - Modal: lần bấm ngoài đầu tiên chỉ đóng menu; phần tử bên dưới không phản ứng (`Menu/MenuContentImpl.vue:45-50`; `Menu/MenuRootContentModal.vue:31`).
  - Focus ra ngoài: modal → bỏ qua. Non-modal → đóng (`Menu/MenuRootContentModal.vue:34`).
  - Chỉ đóng một submenu: phím đóng; highlight sang item khác; focus rời submenu, trừ khi sang SubTrigger của nó hoặc filter của cha; cha đổi trạng thái (`Menu/MenuSubContent.vue:75-86,94-112`; `Menu/MenuSub.vue:52-57`).
  - Modal khi đang hiện: ngoài menu không nhận pointer, trang không cuộn, phần còn lại ẩn với AT (`Menu/MenuRootContentModal.vue:21-23,31`; `Menu/MenuContentImpl.vue:133-136,159-166`).
- **ARIA và look:**
  - Content `role=menu`, `aria-orientation=vertical`. SubContent có `aria-labelledby` = SubTrigger (`Menu/MenuContentImpl.vue:430-433`; `Menu/MenuSubContent.vue:64`).
  - Item disabled: `aria-disabled=true`, vẫn trong cây AT (`Menu/MenuItemImpl.vue:84`).
  - CheckboxItem `aria-checked` = `true` / `false` / `mixed`. RadioItem `aria-checked` = `true` / `false` (`Menu/MenuCheckboxItem.vue:52`; `Menu/MenuRadioItem.vue:50`).
  - SubTrigger: `aria-haspopup=menu`, `aria-expanded`. `aria-controls` = submenu chỉ khi mở và submenu đã vẽ (`Menu/MenuSubTrigger.vue:157-159`; `shared/useTriggerId.ts:215-218`).
  - Group `role=group`, `aria-labelledby` = Label trong nó (`Menu/MenuGroup.vue:25-29`). Separator `role=separator`, `aria-orientation=horizontal` (`Menu/MenuSeparator.vue:16-17`).
  - Modal khi hiện: phần còn lại của trang `aria-hidden` (`Menu/MenuRootContentModal.vue:21-23`).
  - Look cần đọc: content mở/đóng (vào và ra), cạnh và căn thực tế sau flip, gốc biến đổi hướng về anchor, kích thước còn trống tới biên, kích thước anchor; item highlighted, disabled; checked (checked / unchecked / indeterminate, radio chỉ hai giá trị đầu); SubTrigger đang mở; dir. Focus của chính container không bao giờ hiện vòng focus (`RovingFocus/RovingFocusGroup.vue:157`).
- **RTL:**
  - Phím mở submenu là `ArrowLeft`, phím đóng là `ArrowRight` (`Menu/utils.ts:12-19`).
  - Submenu mở bên trái (`Menu/MenuSubContent.vue:66`).
  - Grace dùng cạnh thực tế, đúng cả hai chiều (`Menu/MenuSubTrigger.vue:91-96`).
  - `ArrowUp`/`ArrowDown`/`Home`/`End` không đổi.
- **Cơ chế DOM không port:**
  - Teleport/Portal (`Menu/MenuPortal.vue:14`) → content nằm trên mọi lớp, không bị cắt.
  - Focus guard ở hai mép `body` (`Menu/MenuContentImpl.vue:133`; `shared/useFocusGuards.ts:15-39`) → sự kiện focus vào/ra bắt được nhất quán.
  - `pointer-events: none` trên `body` (`Menu/MenuRootContentModal.vue:31`) → modal: ngoài menu không phản ứng.
  - Khoá cuộn `body` (`Menu/MenuContentImpl.vue:136`) → modal: trang không cuộn.
  - `aria-hidden` các phần tử anh em (`Menu/MenuRootContentModal.vue:23`) → modal: AT chỉ thấy menu.
  - Truy vấn `[data-reka-collection-item]` + `closest('[data-reka-menu-content]')` (`Menu/MenuContentImpl.vue:181-187`) → mỗi content chỉ điều hướng item của chính nó.
  - `CustomEvent('menu.itemSelect')` + `nextTick` (`Menu/MenuItem.vue:32-46`) → chọn huỷ được; đóng sau khi handler chạy xong.
  - `pointerup` gọi `click()` (`Menu/MenuItem.vue:59-68`) → kéo-thả-chọn; Firefox không kẹt chế độ chọn chữ.
  - `focus({ preventScroll: true })` khi hover (`Menu/MenuItemImpl.vue:51`) → hover không cuộn.
  - `scrollIntoView({ block: 'nearest' })` (`Menu/MenuContentImpl.vue:204`) → item highlight từ filter luôn trong tầm nhìn.
  - Focus thủ công khi click SubTrigger vì iOS Safari (`Menu/MenuSubTrigger.vue:164-169`) → focus theo click.
  - So `clientX` thay `movementX` vì Safari (`Menu/MenuContentImpl.vue:331-340`) → hướng chuột đúng.
  - `preventDefault` `Escape` trong submenu (`Menu/MenuSubContent.vue:90-91`) → không thoát fullscreen.
  - `Presence` + `v-show` có điều kiện (`Menu/MenuContent.vue:47-60`; `shared/vShowWhenKept.ts:31-63`) → hoạt ảnh ra; giữ state khi đóng.
  - `useTriggerId` / `useContentId` / `useRenderedContentId` (`shared/useTriggerId.ts:44-218`) → nhãn và `aria-controls` chỉ trỏ tới phần tử có thật.
  - `RovingFocusGroup` bọc content (`Menu/MenuContentImpl.vue:413-425`) → chỉ để có entry focus và danh sách typeahead.
- **Docs ≠ source:**
  - Docs của 3 wrapper nói "roving tabindex". Source: item luôn `tabindex -1`, content không có tab stop nào (`docs:components/dropdown-menu.md:900`; `docs:components/context-menu.md:799`; `docs:components/menubar.md:846`; `Menu/MenuItemImpl.vue:80`).
  - Docs: Item có trạng thái orientation. Source không đặt (`docs:components/dropdown-menu.md:222`; `Menu/MenuItemImpl.vue:77-88`).
  - Docs: RadioItem có indeterminate. Source chỉ checked/unchecked (`docs:components/dropdown-menu.md:287`; `Menu/MenuRadioItem.vue:39-41`).
  - Docs: content orientation ngang hoặc dọc. Source luôn dọc (`docs:components/dropdown-menu.md:161`; `Menu/MenuContentImpl.vue:417`).
- **Dễ sót:**
  - `Escape` trong submenu đóng cả cây. Chỉ phím mũi tên hướng đóng một tầng (`Menu/MenuSubContent.vue:87-112`).
  - Highlight = focus DOM với chuột. Rời item → container giữ focus, không item nào highlight (`Menu/MenuItemImpl.vue:38-72`; `Menu/MenuContentImpl.vue:360-370`).
  - Submenu ở lại khi chuột rời mọi item. Chỉ đóng khi item khác được highlight (`Menu/MenuContentImpl.vue:218-229`). Lý do: submenu có thể che trigger, vì nó ưu tiên nằm trong viewport (`Menu/MenuSubContent.vue:29-31`; `Popper/PopperContent.vue:183-189`).
  - Item đầu chỉ được focus khi input cuối là phím. `pointermove` cũng xoá cờ này (`Menu/MenuContentImpl.vue:420-424`; `shared/useIsUsingKeyboard.ts:14-16`).
  - `ArrowUp` từ container nhảy tới item cuối dù không loop (`shared/useArrowNavigation.ts:177-187`). `PageUp`/`PageDown` chỉ có tác dụng từ container (`Menu/MenuContentImpl.vue:304-313`).
  - `Space` không bao giờ vào typeahead, kể cả giữa một chuỗi typeahead đang gõ. Nó chọn item đang focus, trừ khi filter có chữ (`Menu/MenuContentImpl.vue:291-293`; `Menu/MenuItem.vue:71-82`; `DropdownMenu/DropdownMenuFilter.vue:47-48`).
  - Checked của checkbox và radio do caller giữ. Không lưu thì không đổi (`Menu/MenuCheckboxItem.vue:43`; `Menu/MenuRadioGroup.vue:45`).
  - Chọn checkbox hoặc radio vẫn đóng menu, trừ khi bị huỷ (`Menu/MenuItem.vue:39-44`).
  - Indeterminate + chọn → `true`, không phải `false` (`Menu/MenuCheckboxItem.vue:57-59`).
  - Submenu mở theo di chuột, không theo vào; 100 ms. Click khi đang mở không đóng (`Menu/MenuSubTrigger.vue:73-79,170`).
  - Mở lại cha không bao giờ thấy submenu đang mở (`Menu/MenuSub.vue:52-57`).
  - Content giữ-mounted khi ẩn: không nhốt focus, không khoá cuộn, không ẩn trang với AT, không nhận phím (`Menu/MenuContentImpl.vue:133-136,155-175`; `Menu/MenuRootContentModal.vue:21-23`).
  - Label ngoài Group không đặt tên cho gì. Group không Label vẫn trỏ `aria-labelledby` tới id không tồn tại (`Menu/MenuLabel.vue:15-21`; `Menu/MenuGroup.vue:28`). S6 chỉ đặt tên khi có Label.
  - Item disabled vẫn đọc được với AT nhưng bị phím, typeahead, hover bỏ qua (`Menu/MenuContentImpl.vue:185`; `Collection/Collection.ts:53`).

---

### DropdownMenu
`DropdownMenu/` · dùng bởi: shadcn-vue `dropdown-menu`. Content trong Portal, căn `start`, lệch 4 px. SubContent không Portal (`shadcn:dropdown-menu/DropdownMenuContent.vue:19-20,31`; `shadcn:dropdown-menu/DropdownMenuSubContent.vue`).

- **Part:** Root; Trigger (`button`); Portal; Content; Filter (tuỳ chọn, `role=searchbox`; shadcn-vue không dùng); và mọi part item của Menu (`DropdownMenu/index.ts:1-78`).
- **Trạng thái:**
  - Mở/đóng. Người dùng ghi: click hoặc phím trên Trigger, chọn, dismiss. Caller ghi (điều khiển), hoặc để tự giữ với giá trị đầu (mặc định đóng). Modal mặc định bật (`DropdownMenu/DropdownMenuRoot.vue:33-51`).
  - Mở/đóng submenu: tự giữ với giá trị đầu, hoặc caller điều khiển (`DropdownMenu/DropdownMenuSub.vue:29-32`).
  - Chữ của Filter: tự giữ (rỗng) hoặc caller điều khiển. Người dùng gõ; `Escape` xoá. Filter **không** tự lọc item: caller lọc (`DropdownMenu/DropdownMenuFilter.vue:38-48,82-90`; `docs:components/dropdown-menu.md:816-870`).
  - Trigger disabled: caller.
- **Bàn phím:** Trigger (`DropdownMenu/DropdownMenuTrigger.vue:63-73`):

| Phím | Kết quả |
|---|---|
| `Enter` / `Space` | bật/tắt. Khi mở → item đầu được focus |
| `ArrowDown` | mở (không tắt). Item đầu được focus |
| `ArrowUp` | không làm gì |

Content: như Menu. Filter, khi focus ở ô nhập (`DropdownMenu/DropdownMenuFilter.vue:92-116`; `Menu/MenuContentImpl.vue:189-212`):

| Phím | Kết quả |
|---|---|
| `ArrowDown` / `ArrowUp` / `Home` / `End` | dời highlight, focus ở lại ô nhập. Item được cuộn vào tầm nhìn |
| `Enter` | kích hoạt item đang highlight |
| `Escape` | filter có chữ → xoá chữ, menu ở lại. Filter rỗng → đóng menu |
| phím trong lúc IME đang soạn | không điều hướng, không typeahead |

- **Focus:**
  - Mở: như Menu.
  - Đóng: focus về Trigger (sau 0 ms). Trừ khi trong lúc đó focus đã rời content sang nơi khác, vd một lớp vừa mở cùng cú click. Trừ khi đóng do tương tác ngoài: non-modal là mọi tương tác ngoài; modal là chuột phải (hoặc Ctrl + nút trái) ngoài (`DropdownMenu/DropdownMenuContent.vue:33-48,80-88`).
  - Cùng luật khi content giữ-mounted bị ẩn (`DropdownMenu/DropdownMenuContent.vue:50-57`).
  - Filter tự focus (nếu caller bật) 1 ms sau khi vẽ. Trong submenu chỉ khi input cuối là phím (`DropdownMenu/DropdownMenuFilter.vue:56-66`).
- **Pointer:**
  - Trigger bật/tắt khi **click** (lúc nhả), chỉ nút trái, không Ctrl. Ctrl + click trên macOS là chuột phải (`DropdownMenu/DropdownMenuTrigger.vue:49-62`). Vì mở lúc nhả, không có nhấn-Trigger-kéo-thả-chọn.
  - Non-modal: bấm hoặc focus lên Trigger khi đang mở không tính là "ngoài"; click của Trigger tự đóng menu (`DropdownMenu/DropdownMenuContent.vue:87`).
  - Modal: mọi thứ ngoài content, kể cả Trigger, không nhận pointer khi mở. Bấm lên Trigger là bấm ngoài, menu đóng, focus về Trigger (`Menu/MenuRootContentModal.vue:31`; `DismissableLayer/DismissableLayer.vue:171-195,231-237`; `DropdownMenu/DropdownMenuContent.vue:33-48`).
  - Có filter: hover item chỉ highlight, focus ở lại ô nhập (`Menu/MenuItemImpl.vue:49-51`).
- **Đóng:** như Menu. Thêm: click Trigger khi đang mở.
- **ARIA và look:**
  - Trigger: `aria-haspopup=menu`, `aria-expanded`. `aria-controls` = content chỉ khi mở và content đã vẽ. Disabled thật: không focus, không kích hoạt (`DropdownMenu/DropdownMenuTrigger.vue:38-47`).
  - Content: `aria-labelledby` = Trigger (`DropdownMenu/DropdownMenuContent.vue:65-66`).
  - Filter: `role=searchbox`. `aria-activedescendant` = id của item đang highlight (`DropdownMenu/DropdownMenuFilter.vue:53-54,130`).
  - Look cần đọc: như Menu, cộng Trigger mở/đóng và disabled. shadcn-vue: content rộng bằng Trigger, tối thiểu 8rem; cao tối đa bằng chiều cao còn trống (`shadcn:dropdown-menu/DropdownMenuContent.vue:35`; `shadcn-style:style-vega.css:501-504`). Item sáng theo focus, tức là theo highlight (`shadcn-style:style-vega.css:509-511`).
- **RTL:** như Menu. Phím của Trigger không đổi.
- **Cơ chế DOM không port:**
  - `setTimeout(0)` + `hasFocusLeftScope` (`DropdownMenu/DropdownMenuContent.vue:37-42`) → không cướp focus của một lớp vừa mở cùng cú click.
  - `preventDefault` sau `nextTick` trên click (`DropdownMenu/DropdownMenuTrigger.vue:55-59`) → Trigger không giành focus với content.
  - `type=button` (`DropdownMenu/DropdownMenuTrigger.vue:40`) → không submit form.
  - `stopPropagation` khi IME soạn (`DropdownMenu/DropdownMenuFilter.vue:98-101`) → IME giữ phím của nó.
  - `stopPropagation` `Escape` khi filter có chữ (`DropdownMenu/DropdownMenuFilter.vue:110-115`) → `Escape` xoá chữ mà không đóng.
- **Docs ≠ source:**
  - Docs: `Enter`/`Space` trên Trigger "mở". Source: bật/tắt (`docs:components/dropdown-menu.md:910,921`; `DropdownMenu/DropdownMenuTrigger.vue:66`).
  - Docs liệt kê biến look dưới SubTrigger. Source đặt chúng trên SubContent (`docs:components/dropdown-menu.md:353`; `DropdownMenu/DropdownMenuSubContent.vue:26-36`).
- **Dễ sót:**
  - Mở lúc click (nhả), không lúc nhấn (`DropdownMenu/DropdownMenuTrigger.vue:49-62`).
  - `ArrowUp` trên Trigger không mở (`DropdownMenu/DropdownMenuTrigger.vue:63`).
  - Đóng do tương tác ngoài → focus không về Trigger (`DropdownMenu/DropdownMenuContent.vue:80-88`).
  - `aria-activedescendant` của filter cần item có id. Item không có id mặc định, nên giá trị rỗng; ví dụ trong docs tự gán id (`DropdownMenu/DropdownMenuFilter.vue:54`; `Menu/MenuItemImpl.vue:77-86`; `docs:components/dropdown-menu.md:858-861`). S6 phải tự gán.
  - Đóng submenu bằng phím khi cha có filter → focus về filter, SubTrigger được highlight (`Menu/MenuSubContent.vue:100-105`).

---

### ContextMenu
`ContextMenu/` · dùng bởi: shadcn-vue `context-menu`. Content trong Portal; SubContent không Portal (`shadcn:context-menu/ContextMenuContent.vue:25-36`; `shadcn:context-menu/ContextMenuSubContent.vue`).

- **Part:** Root; Trigger = một vùng (mặc định `span`), không role, tự nó không focus được (`ContextMenu/ContextMenuTrigger.vue:27-30,137-155`); Portal; Content; và mọi part item của Menu.
- **Trạng thái:**
  - Mở/đóng. Người dùng ghi: chuột phải, nhấn giữ, phím menu ngữ cảnh của nền tảng, chọn, dismiss. Caller ghi (điều khiển) hoặc để tự giữ (mặc định đóng). Modal mặc định bật (`ContextMenu/ContextMenuRoot.vue:39-53`).
  - Điểm neo. Mở bằng pointer → điểm con trỏ. Caller mở → cả vùng Trigger. Neo được giữ trong lúc đóng (không nhảy khi hoạt ảnh ra) (`ContextMenu/ContextMenuTrigger.vue:35-68`).
  - Thời gian nhấn giữ: 700 ms, caller đổi được (`ContextMenu/ContextMenuRoot.vue:16-23,41`).
  - Trigger disabled (caller): không mở; menu của hệ thống hiện lại (`ContextMenu/ContextMenuTrigger.vue:5-12,94`).
- **Bàn phím:** Trigger:

| Phím | Kết quả |
|---|---|
| mọi phím | reka không có handler phím; nó chỉ nghe sự kiện `contextmenu` (`ContextMenu/ContextMenuTrigger.vue:93-102,150`). Mở bằng phím chỉ khi nền tảng gửi sự kiện đó cho một phần tử trong vùng. Khi đó item đầu được focus, vì input cuối là phím |

Content: như Menu.
- **Focus:**
  - Mở: như Menu.
  - Đóng: focus về phần tử đã có focus trước khi mở (`FocusScope/FocusScope.vue:284-291`; `ContextMenu/ContextMenuContent.vue:47-53`).
  - Non-modal + tương tác ngoài → focus ở lại chỗ người dùng bấm (`ContextMenu/ContextMenuContent.vue:74-81,89-90`).
- **Pointer:**
  - Chuột phải → mở tại con trỏ. Menu của hệ thống bị chặn (`ContextMenu/ContextMenuTrigger.vue:93-102`).
  - Chuột phải lần nữa trong vùng khi đang mở → menu dời tới điểm mới. Không đóng rồi mở lại (`ContextMenu/ContextMenuTrigger.vue:78-82`; `ContextMenu/ContextMenuContent.vue:84-88`; `ContextMenu/ContextMenu.browser.test.ts:40-50`).
  - Vùng Trigger nhận pointer khi đang mở, kể cả khi modal chặn phần còn lại (`ContextMenu/ContextMenuTrigger.vue:145-147`).
  - Chuột phải vào vùng Trigger khác khi đang mở → menu này đóng, menu kia mở, không có menu hệ thống (`ContextMenu/ContextMenu.browser.test.ts:28-38`).
  - Vùng Trigger đang đóng, nằm sau một dialog modal → không bắt chuột phải (`ContextMenu/ContextMenu.browser.test.ts:52-68`).
  - Chạm hoặc bút: nhấn giữ 700 ms → mở tại điểm nhấn. **Mọi** `pointermove`, `pointerup`, `pointercancel` đều huỷ; không có ngưỡng dung sai (`ContextMenu/ContextMenuTrigger.vue:104-122,151-154`).
  - Nhấn giữ trên iOS không hiện callout (`ContextMenu/ContextMenuTrigger.vue:144`).
  - Nếu menu mở ngay lúc nhấn (nền tảng gửi `contextmenu` lúc nhấn), nhả trên một item sẽ chọn nó, nhờ luật của item (`Menu/MenuItem.vue:59-68`).
- **Đóng:** như Menu. Chuột phải trong vùng Trigger không đóng. Click trái ra ngoài đóng.
- **ARIA và look:**
  - Trigger: không role, không `aria-haspopup`, không `aria-expanded` (`ContextMenu/ContextMenuTrigger.vue:137-155`).
  - Content `role=menu` **không** có `aria-labelledby`: menu không tên (`ContextMenu/ContextMenuContent.vue:57-73`).
  - Vị trí cố định: bên phải điểm neo, lệch 2 px, căn start, tính lại mỗi khung. Caller không đổi được 4 giá trị đó, cũng như khoảng đệm của mũi tên (`ContextMenu/ContextMenuContent.vue:10-18,57-61`; `Popper/PopperContent.vue:365`).
  - Look cần đọc: như Menu, cộng vùng Trigger mở/đóng và disabled.
- **RTL:** như Menu. Cạnh mở vẫn là **phải** cả khi RTL (`ContextMenu/ContextMenuContent.vue:58`).
- **Cơ chế DOM không port:**
  - Phần tử ảo 0×0 tại `clientX`/`clientY` (`ContextMenu/ContextMenuTrigger.vue:39-56`) → neo tại con trỏ.
  - `pointer-events: auto` trên Trigger khi mở (`ContextMenu/ContextMenuTrigger.vue:145-147`) → chuột phải lần hai dời menu.
  - `-webkit-touch-callout: none` (`ContextMenu/ContextMenuTrigger.vue:144`) → không callout iOS.
  - `preventDefault` của `contextmenu` (`ContextMenu/ContextMenuTrigger.vue:99`) → không có menu hệ thống.
  - Cờ "đang chờ mở bằng pointer" (`ContextMenu/ContextMenuTrigger.vue:37-38,60-68,84-91`) → neo là điểm con trỏ chỉ khi chính lần mở đó đến từ pointer. Caller từ chối thì lần mở sau của caller neo vào vùng.
- **Docs ≠ source:**
  - Docs: mở được bằng "phím tắt liên quan". Source không có handler phím; chỉ có sự kiện `contextmenu` của trình duyệt (`docs:components/context-menu.md:114`; `ContextMenu/ContextMenuTrigger.vue:150-154`).
- **Dễ sót:**
  - Mọi `pointermove` khi chạm đều huỷ nhấn giữ (`ContextMenu/ContextMenuTrigger.vue:116-122`).
  - Chuột phải trong vùng khi mở = dời menu (`ContextMenu/ContextMenuTrigger.vue:78-82`).
  - Không có Trigger để trả focus; focus về phần tử trước đó.
  - Content không có tên với AT.
  - Caller mở → neo vào cả vùng, không vào điểm (`ContextMenu/ContextMenuTrigger.vue:52-56`; `ContextMenu/ContextMenu.test.ts:80-130`).
  - Cạnh phải cố định, kể cả RTL.

---

### Menubar
`Menubar/` · dùng bởi: shadcn-vue `menubar`. Content và SubContent đều trong Portal. Content căn `start`, lệch 8 px, lệch căn −4 px (`shadcn:menubar/MenubarContent.vue:19-21,31`; `shadcn:menubar/MenubarSubContent.vue:25`).

- **Part:** Root `role=menubar` (`Menubar/MenubarRoot.vue:99-116`); Menu = một menu của thanh, không vẽ, là một engine Menu non-modal (`Menubar/MenubarMenu.vue:56-72`); Trigger `role=menuitem` (`button`) (`Menubar/MenubarTrigger.vue:43-103`); Portal, Content, SubTrigger, SubContent và mọi part item của Menu.
- **Trạng thái:**
  - Menu đang mở của thanh: một chuỗi. Rỗng = không menu nào. Tối đa một menu mở. Người dùng ghi: nhấn trigger, hover khi thanh đang mở, phím, dismiss, chọn. Caller ghi (điều khiển) hoặc để tự giữ với giá trị đầu (`Menubar/MenubarRoot.vue:69-96`; `Menubar/MenubarMenu.vue:40,62-68`).
  - Giá trị mỗi menu: caller đặt hoặc tự sinh (`Menubar/MenubarMenu.vue:33`).
  - Tab stop của thanh: trigger được focus hoặc mở gần nhất (`Menubar/MenubarRoot.vue:74,83-95`; `RovingFocus/RovingFocusItem.vue:141`).
  - "Mở bằng phím từ trigger": theo từng menu, xoá khi menu đóng (`Menubar/MenubarMenu.vue:38-45`).
  - Chỉ caller cấu hình: loop (mặc định không) cho cả trigger lẫn chuyển menu; dir; giữ-mounted; trigger disabled (`Menubar/MenubarRoot.vue:53-56`).
  - Luôn non-modal (`Menubar/MenubarMenu.vue:59`).
- **Bàn phím:** Trigger (`Menubar/MenubarTrigger.vue:84-94`; `RovingFocus/RovingFocusItem.vue:61-101`; `RovingFocus/utils.ts:9-43`):

| Phím | Kết quả |
|---|---|
| `Enter` / `Space` | có menu nào đang mở → đóng hết. Không → mở menu này, item đầu được focus |
| `ArrowDown` | mở menu này, item đầu được focus |
| `ArrowRight` / `ArrowLeft` | trigger kế / trước (RTL đảo), bỏ disabled, loop nếu bật. Không mở menu |
| `Home` / `PageUp`, `End` / `PageDown` | trigger đầu / cuối |
| `ArrowUp` | không làm gì |
| mũi tên kèm Ctrl/Alt/Meta/Shift | không di chuyển |
| `Tab` | rời thanh (cả thanh là một tab stop) |

Content (`Menubar/MenubarContent.vue:52-79,124`):

| Phím | Kết quả |
|---|---|
| `ArrowRight` (LTR) trên item thường | mở menu kế (loop nếu bật). Trên SubTrigger → mở submenu |
| `ArrowLeft` (LTR) | mở menu trước |
| `Escape` | đóng, focus về trigger |
| khác | như Menu |

SubContent (`Menubar/MenubarSubContent.vue:30-50,67`):

| Phím | Kết quả |
|---|---|
| `ArrowRight` trên item không phải SubTrigger | mở menu kế của thanh. Phím cứng, cả khi RTL |
| `ArrowLeft` (LTR) | đóng submenu (luật Menu) |

- **Focus:**
  - Thanh là một tab stop. Tab vào → trigger đang là tab stop (`RovingFocus/RovingFocusItem.vue:126`).
  - Mở bằng `Enter` / `Space` / `ArrowDown` trên trigger → item đầu. Mở bằng pointer, hoặc chuyển menu bằng mũi tên trong content → container được focus, chưa item nào (`Menubar/MenubarContent.vue:121-123`; `Menubar/MenubarTrigger.vue:90-93`).
  - Đóng → focus về trigger của nó. Trừ khi một menu khác của thanh đã mở thay. Trừ khi đóng do tương tác ngoài (`Menubar/MenubarContent.vue:98-110`; giữ-mounted: `Menubar/MenubarContent.vue:42-50`).
  - Focus sang một trigger của thanh không đóng menu (`Menubar/MenubarContent.vue:111-115`). `Tab` trong content: focus đi ra → đóng (non-modal).
  - Chọn item → đóng, focus về trigger.
- **Pointer:**
  - Nhấn (nút trái, không Ctrl) trên trigger → mở menu đó **ngay lúc nhấn** (`Menubar/MenubarTrigger.vue:66-76`). Vì vậy nhấn-kéo-thả lên item = chọn (`Menu/MenuItem.vue:59-68`).
  - Nhấn trigger của menu đang mở → lớp dismiss coi là bấm ngoài → menu đóng. Content không loại trigger khỏi "ngoài" với pointer (`Menubar/MenubarContent.vue:116-120`).
  - Khi một menu đang mở: pointer vào trigger khác (mọi loại pointer) → mở menu đó ngay, không trễ. Focus qua trigger đó rồi vào container của content mới, chưa item nào (`Menubar/MenubarTrigger.vue:77-83`; `Menu/MenuContentImpl.vue:253-263`; `Menubar/MenubarContent.vue:121-123`). Khi không menu nào mở: hover không làm gì.
- **Đóng:** như Menu, non-modal: focus ra ngoài đóng, trừ khi sang trigger của thanh. Đóng một menu = cả thanh về rỗng (`Menubar/MenubarMenu.vue:62-68`).
- **ARIA và look:**
  - Root `role=menubar`, không đặt `aria-orientation` (`Menubar/MenubarRoot.vue:110`).
  - Trigger `role=menuitem`, `aria-haspopup=menu`, `aria-expanded`. `aria-controls` khi mở (`Menubar/MenubarTrigger.vue:57-60`).
  - Content `aria-labelledby` = trigger (`Menubar/MenubarContent.vue:86`).
  - Look cần đọc: trigger mở/đóng, highlighted (= đang focus), disabled; content như Menu. vega tô nền trigger theo trạng thái mở; content của menubar không có hoạt ảnh ra, SubContent thì có (`shadcn-style:style-vega.css:758-764,806-808`).
- **RTL:**
  - Phím menu trước / kế đảo (`Menubar/MenubarContent.vue:58-60`). Roving trigger đảo (`RovingFocus/utils.ts:20-28`). Submenu như Menu.
  - Bug reka? SubContent dùng `ArrowRight` cứng. Trong RTL `ArrowRight` vừa đóng submenu vừa mở menu kế; `ArrowLeft` không chuyển menu (`Menubar/MenubarSubContent.vue:67`; `Menu/utils.ts:16-19`).
- **Cơ chế DOM không port:**
  - `RovingFocusGroup` + `RovingFocusItem`, tabindex 0/−1 (`Menubar/MenubarRoot.vue:101-107`) → thanh là một tab stop, nhớ trigger cuối.
  - Collection riêng + `data-value` (`Menubar/MenubarContent.vue:66`) → thứ tự menu khi chuyển bằng mũi tên.
  - `preventDefault` lúc nhấn khi mở (`Menubar/MenubarTrigger.vue:72-74`) → focus vào content, không vào trigger.
  - Dấu SubTrigger (`Menubar/MenubarSubTrigger.vue:18`) → mũi tên kế trên SubTrigger mở submenu, không chuyển menu.
  - Listener riêng trên SubContent vì sự kiện không nổi qua portal (`Menubar/MenubarSubContent.vue:30-50`) → mũi tên kế trong submenu chuyển menu.
  - Id trigger = giá trị menu (`Menubar/MenubarMenu.vue:50`) → nhãn content.
- **Docs ≠ source:**
  - Docs: `Space` "mở menubar". Source: bật/tắt cấp thanh (`docs:components/menubar.md:855`; `Menubar/MenubarRoot.vue:90-95`).
  - Docs không nói `Home` / `End` / `PageUp` / `PageDown` trên trigger. Source có (`RovingFocus/utils.ts:9-18`).
- **Dễ sót:**
  - Mở lúc nhấn, khác DropdownMenu.
  - Hover chuyển menu chỉ khi thanh đang mở; không trễ.
  - Chuyển menu bằng mũi tên → chưa item nào được focus; `ArrowDown` mới tới item đầu.
  - Mũi tên kế trên SubTrigger mở submenu, không chuyển menu (`Menubar/MenubarContent.vue:62-64`).
  - `Enter`/`Space` trên trigger là bật/tắt cấp thanh.
  - Chuyển menu bỏ qua trigger disabled (`Menubar/MenubarContent.vue:66`).
  - Bug reka? Hover-chuyển không xét disabled, khác nhấn và phím (`Menubar/MenubarTrigger.vue:70,77-83,85`). S6: trigger disabled không mở theo hover.

---

### NavigationMenu
`NavigationMenu/` · dùng bởi: shadcn-vue `navigation-menu`. Viewport bật mặc định. Dùng Indicator, Link, Trigger, Content, List, Item; không dùng Sub (`shadcn:navigation-menu/NavigationMenu.vue:14-33`).

- **Part:**
  - Root: landmark navigation (`nav`), giữ giá trị (`NavigationMenu/NavigationMenuRoot.vue:114,231-244`).
  - Sub: một tầng lồng, giá trị riêng (`NavigationMenu/NavigationMenuSub.vue:35-119`).
  - List: `ul`, bọc trong một khung vô hình làm rãnh của Indicator (`NavigationMenu/NavigationMenuList.vue:19-44`).
  - Item: `li`. Phạm vi của phím mũi tên. Ghép Trigger + Content, hoặc một Link (`NavigationMenu/NavigationMenuItem.vue:48-50,147-155`).
  - Trigger: `button`, bật/tắt Content của Item (`NavigationMenu/NavigationMenuTrigger.vue:146-169`).
  - Content: panel. Vẽ tại chỗ hoặc trong Viewport. Là lớp dismiss non-modal. Không role (`NavigationMenu/NavigationMenuContent.vue:52-75`; `NavigationMenu/NavigationMenuContentImpl.vue:193-208`).
  - Link: `a`. Chọn → đóng cả menu (`NavigationMenu/NavigationMenuLink.vue:33-67`).
  - Indicator: trang trí, ẩn với AT, bám trigger đang mở (`NavigationMenu/NavigationMenuIndicator.vue:61-85`).
  - Viewport: bề mặt chung vẽ Content đang mở, kích thước theo nó (`NavigationMenu/NavigationMenuViewport.vue:159-191`).
  - Nội bộ, với mỗi trigger đang mở: một focus proxy (focus được, vô hình) và, khi có Viewport, một nút `aria-owns` (`NavigationMenu/NavigationMenuTrigger.vue:171-181`).
- **Trạng thái:**
  - Item đang mở: một chuỗi. Rỗng = đóng. Người dùng ghi: hover (theo trễ), click / `Space` / `Enter` (ngay), `Escape` / bấm ngoài / chọn Link (→ rỗng). Caller ghi (điều khiển, có hiệu lực ngay) hoặc để tự giữ với giá trị đầu (`NavigationMenu/NavigationMenuRoot.vue:125-128,174-228`).
  - Giá trị trước: nội bộ. Dùng cho hướng chuyển động và để giữ Content cũ trong Viewport khi đóng (`NavigationMenu/NavigationMenuRoot.vue:129`; `NavigationMenu/NavigationMenuContent.vue:40-48`).
  - Giá trị mỗi Item: caller đặt hoặc tự sinh (`NavigationMenu/NavigationMenuItem.vue:57`).
  - Link active: caller (trang hiện tại) (`NavigationMenu/NavigationMenuLink.vue:14-17`).
  - Chỉ caller cấu hình (mặc định): trễ mở hover 200 ms; cửa sổ bỏ trễ 300 ms; orientation ngang; tắt mở bằng click (không); tắt hover (không); không đóng khi rời Content (không); giữ-mounted (không); căn Viewport (giữa) (`NavigationMenu/NavigationMenuRoot.vue:106-115`; `NavigationMenu/NavigationMenuViewport.vue:33-35`).
  - Sub: giá trị riêng (mặc định rỗng), orientation ngang (`NavigationMenu/NavigationMenuSub.vue:35-50`).
- **Bàn phím:** Trigger và Link cấp trên (`NavigationMenu/NavigationMenuItem.vue:103-144`; `NavigationMenu/NavigationMenuTrigger.vue:111-124`):

| Phím | Kết quả |
|---|---|
| `Enter` | bật/tắt (click của nút) |
| `Space` | bật/tắt đúng một lần mỗi lần nhấn; giữ phím (auto-repeat) bị bỏ. Khi đóng, focus ở trigger |
| `ArrowDown` (ngang) / `ArrowRight` LTR, `ArrowLeft` RTL (dọc), khi đang mở | focus phần tử focus được đầu tiên của Content |
| mũi tên bất kỳ (cả hai trục), các trường hợp khác | trigger hoặc Link cấp trên kế / trước theo thứ tự DOM. Không loop. Không theo dir |
| `Home` / `End` | trigger hoặc Link cấp trên đầu / cuối |
| `Tab`, khi đang mở | vào Content, phần tử đầu (qua proxy) |
| `Space` trên Link | reka không xử lý, để trình duyệt (`NavigationMenu/NavigationMenuItem.vue:106-109`; `NavigationMenu/NavigationMenu.test.ts:436-444`) |

Content (`NavigationMenu/NavigationMenuContentImpl.vue:131-178`):

| Phím | Kết quả |
|---|---|
| `Tab` / `Shift+Tab` | qua các tab stop của Content. Hết cuối → phần tử sau trigger (Item kế). Quá đầu → trigger |
| mũi tên / `Home` / `End` | giữa các tab stop của Content, không loop. Bỏ qua khi focus ở ô nhập |
| `Escape` (khi Content là lớp trên cùng, focus ở đâu cũng được) | đóng, focus về trigger. Hover không mở lại tới khi chuột vào trigger lần nữa (`DismissableLayer/DismissableLayer.vue:146-162`) |
| `Enter` trên Link | điều hướng và đóng menu |

- **Focus:**
  - Mở **không** dời focus. Focus ở lại trigger. Content không có focus scope.
  - Thứ tự Tab: Content đang mở đứng ngay sau trigger của nó, dù vẽ ở Viewport. Tab từ trigger → phần tử đầu. Shift+Tab từ Item kế → phần tử cuối của Content (`NavigationMenu/NavigationMenuTrigger.vue:134-143`; `NavigationMenu/NavigationMenuItem.vue:64-72`; `NavigationMenu/NavigationMenu.test.ts:71-80`).
  - Focus rời Content sang chỗ không phải trigger, hoặc Content bị dismiss → mọi phần tử focus được của nó ra khỏi thứ tự Tab. Vào lại bằng phím vào hoặc qua proxy thì khôi phục (`NavigationMenu/NavigationMenuContentImpl.vue:78-94,113-129`; `NavigationMenu/NavigationMenuItem.vue:61-83`; `NavigationMenu/NavigationMenu.browser.test.ts:76-114`).
  - Proxy chỉ có khi item đang mở, nên Tab từ trigger không đi vào Content đã đóng (`NavigationMenu/NavigationMenuTrigger.vue:171`).
  - `Escape` → focus về trigger. Chọn Link → focus về trigger, chỉ khi focus đang trong Content (`NavigationMenu/NavigationMenuContentImpl.vue:113-139`).
- **Pointer:**
  - Hover chỉ với chuột. Chạm = click (`NavigationMenu/NavigationMenuTrigger.vue:63,82`).
  - Chuột di trên trigger khi menu đóng → mở sau 200 ms. Nếu trong 300 ms sau một lần đóng do hover → 150 ms (`NavigationMenu/NavigationMenuRoot.vue:142-163,194-203`; `NavigationMenu/NavigationMenu.test.ts:229-245`).
  - Chuột vào trigger khác khi đang mở → chuyển ngay, không trễ. Lệnh đóng đang chờ bị bỏ (`NavigationMenu/NavigationMenuRoot.vue:154-157,195-199`; `NavigationMenu/NavigationMenu.test.ts:246-256`).
  - Rời trigger → đóng sau 150 ms. Vào Content hoặc Viewport → huỷ lệnh đóng. Rời Content hoặc Viewport (chuột) → đóng sau 150 ms, trừ khi caller tắt "đóng khi rời" (`NavigationMenu/NavigationMenuRoot.vue:204-216`; `NavigationMenu/NavigationMenuContent.vue:69-70`; `NavigationMenu/NavigationMenuViewport.vue:185-186`; `NavigationMenu/NavigationMenu.test.ts:259-281`).
  - Mọi yêu cầu hover dùng chung một hẹn giờ: yêu cầu sau thay yêu cầu trước. Rời trigger trước khi kịp mở = huỷ lần mở (`NavigationMenu/NavigationMenuRoot.vue:151-163,204-207`).
  - Click chuột bị bỏ qua nếu trong 300 ms trước đó một lần di chuột trên trigger đã gửi yêu cầu mở. Cờ tự xoá sau 300 ms; lần di kế tiếp đặt lại nó. Ý định: menu vừa mở bằng hover không bị click đóng ngay. Hệ quả: click trong lúc chuột còn di trên trigger thường bị bỏ (`NavigationMenu/NavigationMenuTrigger.vue:40,59-76,100-102`).
  - Sau `Escape`: di chuột trên trigger không mở lại, tới khi chuột rời rồi vào lại (`NavigationMenu/NavigationMenuTrigger.vue:51-57,64-71`; `NavigationMenu/NavigationMenuContentImpl.vue:137`).
  - Tắt click: chặn click chuột; chạm và bút vẫn bật/tắt (`NavigationMenu/NavigationMenuTrigger.vue:93-98`). Tắt hover: trigger không mở, không đóng theo chuột (`NavigationMenu/NavigationMenuTrigger.vue:51-88`). Nhưng rời Content hoặc Viewport bằng chuột vẫn đóng sau 150 ms (`NavigationMenu/NavigationMenuContent.vue:70`; `NavigationMenu/NavigationMenuViewport.vue:186`; `NavigationMenu/NavigationMenuRoot.vue:211-216`).
  - Sub: hover trigger con → chuyển ngay. Rời không đóng (`NavigationMenu/NavigationMenuSub.vue:86-97`).
  - Content và Viewport đang đóng không nhận pointer (`NavigationMenu/NavigationMenuContent.vue:64-66`; `NavigationMenu/NavigationMenuViewport.vue:177-178`).
- **Đóng:**
  - `Escape` (`NavigationMenu/NavigationMenuContentImpl.vue:131-139`).
  - Bấm ngoài. Trừ: bấm lên trigger hoặc Link của nav, bấm trong Viewport gốc. Content của Sub không bao giờ đóng vì bấm ngoài (`NavigationMenu/NavigationMenuContentImpl.vue:96-111`).
  - Focus ra khỏi cả nav. Focus sang trigger hoặc chỗ khác trong nav không đóng (`NavigationMenu/NavigationMenuContentImpl.vue:78-94`).
  - Chọn Link: sự kiện huỷ được. Không huỷ → đóng cả menu. Meta + click không đóng (`NavigationMenu/NavigationMenuLink.vue:33-53`).
  - Click hoặc `Space` trên trigger đang mở. Hover rời (có trễ).
- **ARIA và look:**
  - Root là landmark navigation, không tự có tên (`NavigationMenu/NavigationMenuRoot.vue:231-244`). Cố ý không dùng role menu hay menubar (`docs:components/navigation-menu.md:596-603`).
  - Trigger: `aria-expanded`. `aria-controls` = Content chỉ khi Content đã vẽ. Không `aria-haspopup` (`NavigationMenu/NavigationMenuTrigger.vue:155-156`).
  - Content: `aria-labelledby` = trigger (`NavigationMenu/NavigationMenuContentImpl.vue:196`).
  - Link active: `aria-current=page` (`NavigationMenu/NavigationMenuLink.vue:61`).
  - Indicator: `aria-hidden=true` (`NavigationMenu/NavigationMenuIndicator.vue:69`).
  - Có Viewport: một nút `aria-owns` = Content ngay sau trigger, để AT thấy Content tại chỗ trigger (`NavigationMenu/NavigationMenuTrigger.vue:177-180`). Focus proxy không `aria-hidden` (`NavigationMenu/NavigationMenu.test.ts:62-69`).
  - Hướng chuyển động: đổi item → Content mới vào từ phía item mới (từ cuối nếu item mới nằm sau, từ đầu nếu trước). Content cũ ra phía ngược lại. Mở lần đầu và đóng hẳn: không có hướng. RTL đảo thứ tự, nên hướng vẫn theo trái/phải hình học (`NavigationMenu/NavigationMenuContentImpl.vue:43-76`).
  - Viewport: kích thước = Content đang mở, đo lại khi nó đổi. Vị trí theo trigger đang mở, căn đầu / giữa / cuối, kẹp cách mép tài liệu 10 px, làm tròn px. Xoá kích thước và vị trí sau hoạt ảnh ra (`NavigationMenu/NavigationMenuViewport.vue:66-132,160-167`). Viewport giữ Content cũ trong lúc hoạt ảnh ra (`NavigationMenu/NavigationMenuContent.vue:40-48`).
  - Indicator: hiện khi có item mở. Kích thước và vị trí theo trục của list: ngang = rộng và trái của trigger; dọc = cao và trên. Cập nhật khi trigger hoặc list đổi kích thước. Giữ vị trí cuối khi ẩn (`NavigationMenu/NavigationMenuIndicator.vue:30-58`).
  - Look cần đọc: trigger mở/đóng, disabled; Content mở/đóng, hướng chuyển động, orientation; Viewport mở/đóng và kích thước (shadcn-vue chỉ đọc rộng và cao, vị trí do layout) (`shadcn:navigation-menu/NavigationMenuViewport.vue:19-25`); Indicator hiện/ẩn, kích thước, vị trí; Link active; nav có Viewport hay không.
- **RTL:**
  - Thứ tự cho hướng chuyển động đảo (`NavigationMenu/NavigationMenuContentImpl.vue:46-47`).
  - Phím vào Content ở dọc đổi thành `ArrowLeft` (`NavigationMenu/NavigationMenuTrigger.vue:114`).
  - Bug reka? Mũi tên giữa các Item và trong Content **không** theo dir: `ArrowRight` luôn tới phần tử DOM kế (`NavigationMenu/NavigationMenuItem.vue:134-137`; `NavigationMenu/NavigationMenuContentImpl.vue:171-176`; `shared/useArrowNavigation.ts:90`).
- **Cơ chế DOM không port:**
  - Teleport Content vào Viewport (`NavigationMenu/NavigationMenuContent.vue:52-55`) → Content vẽ ở bề mặt chung.
  - Focus proxy `VisuallyHidden` `tabindex=0` (`NavigationMenu/NavigationMenuTrigger.vue:171-176`) → Content đứng sau trigger trong thứ tự Tab.
  - Nút `aria-owns` (`NavigationMenu/NavigationMenuTrigger.vue:177-180`) → AT đọc Content tại trigger.
  - `removeFromTabOrder` + TreeWalker tìm tabbable (`NavigationMenu/utils.ts:32-83`) → Content đóng không Tab vào được.
  - ResizeObserver, `offsetWidth`/`offsetLeft`, `requestAnimationFrame` tìm Content đang mở (`NavigationMenu/NavigationMenuViewport.vue:54-64,124-156`; `NavigationMenu/NavigationMenuIndicator.vue:35-58`) → kích thước Viewport và Indicator.
  - Bù scale của CSS transform (`NavigationMenu/NavigationMenuViewport.vue:71-78`) → vị trí đúng khi nav bị scale.
  - `CustomEvent` dismiss nổi lên tới Content gốc (`NavigationMenu/NavigationMenuLink.vue:43-52`; `NavigationMenu/NavigationMenuContentImpl.vue:113-129`) → chọn Link ở tầng sâu đóng cả cây.
  - `pointer-events: none` khi đóng (`NavigationMenu/NavigationMenuContent.vue:64-66`) → Content đang ra không bắt chuột.
  - Thuộc tính `hidden` khi giữ-mounted (`NavigationMenu/NavigationMenuContent.vue:68`) → Content đóng không hiện, không vào cây AT.
  - `type=button` (`NavigationMenu/NavigationMenuTrigger.vue:159`) → không submit form.
- **Docs ≠ source:**
  - Docs: trạng thái Viewport là visible/hidden. Source: open/closed (`docs:components/navigation-menu.md:245`; `NavigationMenu/NavigationMenuViewport.vue:174`).
  - Docs chỉ nêu rộng và cao của Viewport. Source còn vị trí trái, trên và căn (`docs:components/navigation-menu.md:257-261`; `NavigationMenu/NavigationMenuViewport.vue:11-15,179-182`).
  - Docs: `Space` / `Enter` "mở". Source: bật/tắt (`docs:components/navigation-menu.md:617-620`; `NavigationMenu/NavigationMenuItem.vue:114-123`).
  - Docs: trong cửa sổ bỏ trễ thì "không bị trễ lại". Source vẫn trễ 150 ms (`meta:NavigationMenuRoot.md:72`; `NavigationMenu/NavigationMenuRoot.vue:144-149`).
  - Docs gọi trễ mở là của "tooltip", chép nhầm (`meta:NavigationMenuRoot.md:26`).
  - Docs: tắt "đóng khi rời" = không đóng khi pointer rời. Source chỉ bỏ đóng khi rời Content và Viewport; rời trigger vẫn đóng (`meta:NavigationMenuRoot.md:53`; `NavigationMenu/NavigationMenuRoot.vue:204-216`).
- **Dễ sót:**
  - Chuyển trigger khi đang mở là tức thì. Trễ chỉ áp cho lần mở đầu.
  - Trễ đóng 150 ms để kịp vào Content. Vào Content huỷ lệnh đóng.
  - Mở không dời focus. Proxy giữ Content ngay sau trigger trong thứ tự Tab.
  - Focus rời Content (không sang trigger) → Content bị rút khỏi thứ tự Tab; vào lại thì khôi phục.
  - Click chuột bị bỏ trong 300 ms sau mỗi lần di chuột gửi yêu cầu mở.
  - Tắt hover không tắt luật đóng khi chuột rời Content.
  - `Escape` đóng Content đang mở dù focus ở đâu, và kéo focus về trigger.
  - Bug reka? Cờ "đóng bằng click" được tính **sau** khi đổi giá trị. Khi tự giữ state, cờ bật khi click **mở**, nên click đóng rồi di chuột trên trigger → mở lại sau 200 ms (`NavigationMenu/NavigationMenuTrigger.vue:41,64-71,104-108`). Ý định (tên cờ, luật `Escape` song song): chặn mở lại tới khi chuột vào lại. S6 nên theo ý định.
  - Mũi tên ở cấp trên nhận cả hai trục bất kể orientation. Ở nav ngang, `ArrowDown` trên trigger đóng = sang trigger kế (`NavigationMenu/NavigationMenuItem.vue:134-137`; `docs:components/navigation-menu.md:630-639`).
  - Dời focus giữa các trigger bằng mũi tên không đóng, không đổi Content đang mở (`NavigationMenu/NavigationMenuContentImpl.vue:82-84`).
  - Meta + click Link không đóng; Ctrl + click vẫn đóng (`NavigationMenu/NavigationMenuLink.vue:43`).
  - Look tham chiếu: vega tô Link active theo giá trị `true`, nhưng reka đặt giá trị rỗng. Trong Chrome, Link active không có look riêng (`shadcn-style:style-vega.css:823-824`; `NavigationMenu/NavigationMenuLink.vue:60`). Nếu S6 hiện look active thì ghi vào verdict.

---

### Toolbar
`Toolbar/` · dùng bởi: không component shadcn-vue nào.

- **Part:** Root `role=toolbar` + `aria-orientation` (mặc định ngang) (`Toolbar/ToolbarRoot.vue:30-57`); Button (`button`) (`Toolbar/ToolbarButton.vue:14-32`); Link (`a`) (`Toolbar/ToolbarLink.vue:12-31`); ToggleGroup `role=group` + ToggleItem là nút bật/tắt với `aria-pressed` (`Toolbar/ToolbarToggleGroup.vue:26-35`; `Toolbar/ToolbarToggleItem.vue:16-28`; `Toggle/Toggle.vue:90`); Separator `role=separator` (`Toolbar/ToolbarSeparator.vue:18-26`).
- **Trạng thái:** tab stop hiện tại, nội bộ = item focus gần nhất. Giá trị của ToggleGroup thuộc ToggleGroup. Chỉ caller cấu hình: orientation (ngang), dir, loop (không), disabled từng item.
- **Bàn phím:** (`RovingFocus/RovingFocusItem.vue:61-101`; `RovingFocus/utils.ts:9-43`)

| Phím | Kết quả |
|---|---|
| `Tab` | vào thanh: item focus gần nhất; chưa có thì item bật đầu. `Tab` lần nữa ra khỏi thanh |
| `Shift+Tab` | ra khỏi thanh |
| `ArrowRight` / `ArrowLeft` (ngang, RTL đảo) hoặc `ArrowDown` / `ArrowUp` (dọc) | item bật kế / trước; loop nếu bật |
| mũi tên trục kia | không làm gì |
| `Home` / `PageUp`, `End` / `PageDown` | đầu / cuối |
| mũi tên kèm Ctrl/Alt/Meta/Shift | không di chuyển |
| `Space` trên Link | kích hoạt Link (`Toolbar/ToolbarLink.vue:24-26`) |
| `Enter` / `Space` trên Button, ToggleItem | hành vi gốc của nút |

- **Focus:** một tab stop, nhớ item cuối (`RovingFocus/RovingFocusGroup.vue:102-116`). Item disabled không focus được, kể cả bằng chuột (`RovingFocus/RovingFocusItem.vue:132-140`). Thanh mà mọi item disabled không phải tab stop (`RovingFocus/RovingFocusGroup.vue:152`; `Toolbar/Toolbar.test.ts:38-70`).
- **Pointer:** không có gì riêng.
- **Đóng:** không áp dụng.
- **ARIA và look:** Separator có orientation **giống** toolbar, không xoay vuông góc. Toolbar ngang → separator ngang, không `aria-orientation` (`Toolbar/ToolbarSeparator.vue:19-23`; `shared/component/BaseSeparator.vue:36-45`). S6 quyết có xoay không và ghi lý do. Look cần đọc: orientation; ToggleItem bật/tắt, disabled.
- **RTL:** chỉ đảo trái/phải ở toolbar ngang (`RovingFocus/utils.ts:20-28`).
- **Cơ chế DOM không port:**
  - tabindex 0/−1 trên item, tabindex của container, cờ "đang Shift+Tab ra" (`RovingFocus/RovingFocusGroup.vue:79,152`; `RovingFocus/RovingFocusItem.vue:65-68,126`) → cả thanh là một tab stop.
  - Listener phím gắn native sau mount (`RovingFocus/RovingFocusItem.vue:103-112`) → handler của trigger lồng bên trong chạy trước và thắng.
  - `preventDefault` lúc `mousedown` (`RovingFocus/RovingFocusItem.vue:132-140`) → item disabled không nhận focus chuột.
- **Docs ≠ source:** Docs: `Tab` "focus item đầu". Source: item focus gần nhất, chưa có mới tới item đầu (`docs:components/toolbar.md:194`; `RovingFocus/RovingFocusGroup.vue:102-113`). Docs không nói `PageUp` / `PageDown`.
- **Dễ sót:**
  - Trigger lồng (vd DropdownMenu trong một Button) giành phím: `ArrowDown` mở menu thay vì dời focus, cả toolbar dọc (`Toolbar/Toolbar.test.ts:150-163`).
  - Separator không xoay.
  - `Space` kích hoạt Link.

---

### Ghi chú chung cho nhóm 4
**Engine `Menu` gồm 4 tầng.**
1. Root: mở/đóng, dir, modal, giữ-mounted, cờ "input cuối là phím".
2. Content: chọn modal hay non-modal (`Menu/MenuContent.vue:43`).
3. ContentImpl: focus scope, lớp dismiss, entry focus, popup, phím, typeahead, grace (`Menu/MenuContentImpl.vue`).
4. Item: highlight, chọn. Sub lặp lại Content với trạng thái riêng.

**Ba wrapper chỉ khác ở đây:**

| | DropdownMenu | ContextMenu | Menubar |
|---|---|---|---|
| Mở bằng | click trái (không Ctrl); `Enter`/`Space` bật/tắt; `ArrowDown` | chuột phải; nhấn giữ 700 ms; phím menu ngữ cảnh qua trình duyệt | nhấn trái; `Enter`/`Space` bật/tắt cấp thanh; `ArrowDown`; hover khi thanh mở; ←/→ trong content |
| Neo | Trigger | điểm con trỏ; caller mở → vùng | Trigger |
| Modal mặc định | có | có | luôn không |
| Item đầu khi mở | nếu input cuối là phím | nếu input cuối là phím | chỉ khi mở bằng phím từ trigger |
| Focus khi đóng | Trigger, trừ tương tác ngoài (non-modal: mọi; modal: chuột phải) | phần tử trước khi mở | Trigger, trừ khi menu khác mở thay hoặc tương tác ngoài |
| Ngoại lệ của "ngoài" | Trigger | chuột phải trong vùng | focus sang trigger của thanh |
| Vị trí mặc định | dưới, giữa | phải, lệch 2 px, start, tính lại mỗi khung | dưới, start |
| Tên với AT | nhãn Trigger | không có | nhãn Trigger |

**Engine cấu hình các khối dựng như sau:**
- Popper: root dùng mặc định của Popper (dưới, giữa, lệch 0, flip, shift, sticky partial, `fixed`) (`Popper/PopperContent.vue:15-33`). Sub: phải hoặc trái theo dir, căn start, ưu tiên nằm trong viewport. ContextMenu và Menubar như bảng trên.
- DismissableLayer: root modal chặn pointer ngoài khi mở; non-modal không. Sub không chặn, và không đóng theo dismiss của chính nó: nó đóng qua phím, focus, highlight, cha (`Menu/MenuSubContent.vue:58-112`).
- FocusScope: modal nhốt focus khi mở; non-modal và sub không. Khi mở → container. Khi đóng → wrapper quyết.
- RovingFocusGroup: trong content chỉ để có entry focus và danh sách typeahead; item không phải roving item. Menubar và Toolbar dùng roving thật cho trigger và item.
- Collection: thứ tự DOM cho typeahead. Mũi tên dùng danh sách item bật của chính content.

**Bất biến S6 cần giữ:**
1. Mỗi content có một highlight. Chuột và phím cùng ghi. Focus đi theo, trừ khi một ô nhập trong menu giữ focus.
2. Mỗi content (gốc, mỗi submenu) có điều hướng và typeahead riêng. Phím trong submenu không chạm cha.
3. Chọn → đóng cả cây, trừ khi caller huỷ.
4. `Escape` → đóng cả cây. Phím hướng → đóng một tầng.
5. Mở bằng phím → item đầu. Mở bằng pointer → không item nào.
6. Submenu đóng khi cha đổi trạng thái.
7. Content gắn nhãn bằng trigger của nó (trừ ContextMenu).

**Reka không có (S6 quyết, ghi lý do nếu thêm):** cờ close-on-select; checkbox/radio tự giữ state; roving tabindex trong content; `Space` trong typeahead; `ArrowUp` mở DropdownMenu; `aria-haspopup` trên vùng ContextMenu; tên cho ContextMenu.

**Bug reka? (S6 theo ý định, ghi lại):** SubContent của Menubar dùng `ArrowRight` cứng (RTL); NavigationMenu bỏ dir khi đi mũi tên; cờ click-đóng của NavigationMenu tính sau khi đổi giá trị; Group không Label vẫn trỏ `aria-labelledby`; `aria-activedescendant` của Filter cần id mà item không có.

## 9. Nhóm 5: chọn và danh sách

### Select
`Select/` · dùng bởi: shadcn-vue `select` (`shadcn:select/`). `native-select` chỉ import một type.

#### Part
- Gốc: giữ giá trị và mở/đóng. Không vẽ gì.
- Trigger: nút mở. Giữ focus khi đóng.
- Value: chữ của item đã chọn, hoặc placeholder. Nằm trong trigger.
- Icon: mũi tên trang trí, AT bỏ qua (`Select/SelectIcon.vue:16-22`).
- Content: popup danh sách. Hai cách đặt vị trí (xem "Vị trí").
- Viewport: vùng cuộn, không thanh cuộn.
- Item; ItemText (chữ dùng cho nhãn trigger, typeahead và căn hàng); ItemIndicator (hiện khi item được chọn).
- Group + Label; Separator.
- Nút cuộn lên, nút cuộn xuống.
- Arrow: chỉ ở chế độ popper, 10 × 5 (`Select/SelectArrow.vue:20-25`).

#### Trạng thái
- **Giá trị**: một giá trị, hoặc một danh sách khi chọn nhiều. Mặc định: không có, hoặc danh sách rỗng (`Select/SelectRoot.vue:112-117`).
  - Người dùng chọn item, chế độ một: thay giá trị và đóng (`Select/SelectItem.vue:84-95`).
  - Người dùng chọn item, chế độ nhiều: bật/tắt thành viên. Item mới thêm vào cuối. Popup vẫn mở (`Select/SelectRoot.vue:163-174`).
  - Người dùng gõ typeahead trên trigger đang đóng: thay giá trị bằng item khớp. Chỉ chế độ một (`Select/SelectTrigger.vue:87-93`).
  - Trình duyệt autofill form: ghi giá trị (`Select/BubbleSelect.vue:41-43`).
  - Caller có thể từ chối từng lần chọn, kể cả lần chọn bằng typeahead khi đóng (`Select/SelectItem.vue:84-89,97-108`).
  - Caller ghi (điều khiển hoặc giá trị đầu): chỉ cập nhật "có giá trị" của `Field`. Ghi của người dùng còn đánh dấu "đã sửa" và chạy kiểm tra khi đổi (`Select/SelectRoot.vue:138-145,176-183`).
  - "Rỗng" = `undefined`, `null`, `''` hoặc `[]` (`Select/utils.ts:33-35`).
  - Item không được có giá trị `''`: ném lỗi, vì `''` nghĩa là xóa chọn (`Select/SelectItem.vue:206-210`).
  - Bằng nhau: chuỗi so `===`; caller có thể cho tên khóa hoặc hàm so; còn lại so sâu (`Select/utils.ts:17-31`).
- **Mở/đóng**: người dùng (trigger, phím, Escape, nhấn ngoài, chọn ở chế độ một) và caller (`Select/SelectRoot.vue:119-122`).
- **Vô hiệu**: gốc (hoặc `Field`) vô hiệu → trigger và mọi item vô hiệu. Item có thể vô hiệu riêng (`Select/SelectRoot.vue:110`, `Select/SelectItem.vue:64`).
- **Highlight**: chính là item đang có focus. Không có trạng thái riêng (`Select/SelectItem.vue:241,249-250`).
- Nội bộ, chỉ người dùng ghi: điểm nhấn trigger (cho ngưỡng 10 px), chuỗi typeahead.

#### Vị trí
- Hai chế độ: căn theo item (mặc định của reka và shadcn) và popper (`Select/SelectContentImpl.vue:108-113`, `shadcn:select/SelectContent.vue:21`).
- **Item neo**: item được focus khi mở, ở cả hai chế độ. Ở chế độ căn theo item, nó nằm đúng trên trigger (`Select/SelectContentImpl.vue:254-280`).
  - Chế độ một: item đang chọn. Không có → item bật đầu tiên.
  - Chế độ nhiều: item được chọn đầu tiên theo thứ tự hiển thị. Không có → item bật đầu tiên.
  - Hệ quả của mã: chế độ một, nếu item đang chọn bị vô hiệu và đứng trước item bật đầu tiên, item bật đầu tiên thắng (`Select/SelectContentImpl.vue:268-279`).
- **Căn theo item** (`Select/SelectItemAlignedPosition.vue:49-193`):
  - Ngang, LTR: mép trái chữ của item neo trùng mép trái chữ Value trong trigger (`:68-79`).
  - Bề rộng tối thiểu = bề rộng trigger + độ lệch đó. Popup luôn phủ trigger (`:71-73,77`).
  - Kẹp để popup cách mép cửa sổ ít nhất 10 px (`:74-75`, `Select/utils.ts:6`).
  - RTL: dùng mép phải (`:80-95`).
  - Dọc: điểm giữa item neo trùng điểm giữa trigger.
  - Phần nội dung phía trên điểm giữa item vừa trong khoảng (mép trên + 10 px → giữa trigger): popup neo đáy, kéo tới cách mép dưới 10 px (`:144-164`).
  - Không vừa: popup neo đỉnh, cách mép trên 10 px. Viewport cuộn sẵn để item neo vẫn nằm giữa trigger (`:165-182`).
  - Item neo luôn hiện trọn, kể cả khi trigger sát mép. Item đầu/cuối tính cả padding của viewport, để không sinh cuộn thừa (`:150-163,166-176`).
  - Cao tối thiểu = min(5 × chiều cao item neo, toàn bộ nội dung). Cao tối đa = cửa sổ − 20 px (`:101,120-123,184-186`).
  - Người dùng cuộn danh sách: khung lớn dần theo lượng cuộn, tới chiều cao khả dụng, rồi mới cuộn thật. Khung neo đáy thì nội dung dính đáy (`Select/SelectViewport.vue:41-69`).
  - Việc lớn dần chỉ bật sau 1 frame, để cú cuộn sẵn lúc mở không tính (`Select/SelectItemAlignedPosition.vue:191-193`).
  - Nút cuộn xuất hiện lần đầu → tính lại vị trí một lần, focus lại item neo (`:211-217`).
  - Trigger đổi kích thước → tính lại (`:220-222`). Không có nguồn tính lại nào khác trong file.
  - shadcn tắt animation mở ở chế độ này (`shadcn:select/SelectContent.vue:36-39`).
- **Popper**: đặt như popover. Mặc định của Select: cạnh dưới, căn đầu, lệch 0, lật khi chạm, cách mép va chạm 10 px (`Select/SelectPopperPosition.vue:12-15`, `Popper/PopperContent.vue:15-33`).
  - shadcn: căn giữa; viewport rộng ít nhất bằng trigger; lệch 4 px theo cạnh (`shadcn:select/SelectContent.vue:22,40-41,50`).
- Cạnh, căn, lệch, va chạm chỉ có tác dụng ở popper (`Select/SelectContentImpl.vue:238-246`). shadcn đặt căn giữa, nhưng ở chế độ căn theo item nó không đổi gì.

#### Bàn phím
Trigger:

| Phím | Kết quả |
|---|---|
| `Space`, `Enter`, `ArrowDown`, `ArrowUp` | Mở (trừ khi vô hiệu). Chặn mặc định. Reset chuỗi typeahead. Khi đặt xong vị trí, focus item neo (`Select/SelectTrigger.vue:94-100,198-201`, `Select/utils.ts:4`, `Select/SelectContentImpl.vue:139-146`). |
| Ký tự in được, không `Ctrl`/`Alt`/`Meta`, khi đóng | Chọn ngay item khớp, không mở. Chỉ chế độ một. Bỏ item vô hiệu (`Select/SelectTrigger.vue:87-93,186-193`). |
| Ký tự in được, khi mở | Typeahead trong danh sách (`Select/SelectTrigger.vue:190-191`). |
| `Space` khi chuỗi typeahead chưa rỗng | Là một ký tự của chuỗi tìm. Không mở (`Select/SelectTrigger.vue:186-196`). |
| `Shift`, mũi tên trái/phải… | Không vào chuỗi tìm. Chỉ ký tự độ dài 1 mới vào (`Select/SelectTrigger.vue:187-189`). |

Content (focus trên item):

| Phím | Kết quả |
|---|---|
| `ArrowDown` / `ArrowUp` | Item bật kế / trước. Dừng ở hai đầu. Chỉ vòng khi bật tùy chọn vòng (mặc định tắt) (`Select/SelectContentImpl.vue:81-85,219-235`). |
| `Home` / `End` | Item bật đầu / cuối (`Select/SelectContentImpl.vue:219-235`). |
| `PageUp` / `PageDown` | Không có xử lý riêng (`Select/SelectContentImpl.vue:219`). |
| `Enter` | Chọn item đang focus. Chế độ một → đóng. Chế độ nhiều → bật/tắt, vẫn mở (`Select/SelectItem.vue:190-204,84-95`). |
| `Space` | Như `Enter` khi chuỗi tìm rỗng. Khi đang gõ dở: là ký tự tìm, không chọn. Luôn chặn cuộn trang (`Select/SelectContentImpl.vue:209-214`, `Select/SelectItem.vue:196-203`). |
| Ký tự in được, không `Ctrl`/`Alt`/`Meta` | Typeahead: focus item khớp, không chọn (`Select/SelectContentImpl.vue:216-217`; test `Select/Select.test.ts:603-611`). |
| `Tab` | Bị chặn. Focus không rời popup (`Select/SelectContentImpl.vue:205-206`). |
| `Escape` | Đóng. Focus về trigger, không cuộn (`Select/SelectContentImpl.vue:302-319`). |

Typeahead (`shared/useTypeahead.ts`): chuỗi tự xóa sau 1000 ms (`:16`). Gõ lặp một chữ → xoay vòng các item bắt đầu bằng chữ đó. Khớp tiền tố, không phân biệt hoa thường. Tìm từ sau item hiện tại (`:78-96`). Chữ của item = chữ caller cho; rỗng thì lấy chữ hiển thị (`:9-12`).

#### Focus
- Mở: focus item neo sau khi đặt xong vị trí. Item được cuộn vào tầm nhìn (`Select/SelectContentImpl.vue:139-146`, `Menu/utils.ts:48-58`).
- Không item nào nhận focus (ví dụ item neo vô hiệu) → focus vào chính popup (`Select/SelectContentImpl.vue:141-142`).
- Nhấn chuột mở: trigger không giật lại focus từ item (`Select/SelectTrigger.vue:144-153`).
- Nhấn label của Select → focus trigger (`Select/SelectTrigger.vue:155-168`).
- Nhấn chuột xuống item → focus item ngay, không cuộn (`Select/SelectItem.vue:128-133`).
- Di chuột trên item → focus item, không cuộn (`Select/SelectItem.vue:168-180`).
- Di vào item vô hiệu, hoặc rời item đang focus → focus về popup. Không còn highlight (`Select/SelectItem.vue:172-173,182-188`, `Select/SelectContentImpl.vue:283-285`).
- Đóng: focus về trigger, không cuộn. Trừ khi focus đã sang nơi khác, ví dụ một lớp vừa mở bởi cùng cú nhấn (`Select/SelectContentImpl.vue:305-313`).
- Phím mũi tên, `Home`/`End`, typeahead chuyển focus và cuộn item mới vào tầm nhìn (`Select/SelectContentImpl.vue:233`, `shared/useTypeahead.ts:32-34`, `Menu/utils.ts:48-58`).
- Nút cuộn xuất hiện → item đang focus được cuộn vào tầm nhìn gần nhất (`Select/SelectScrollButtonImpl.vue:25-30`).

#### Pointer
- Chuột/bút: nhấn trái (không `Ctrl`) xuống trigger → mở ngay (`Select/SelectTrigger.vue:110-112,121-142`).
- Nhấn–kéo–thả: nhấn trigger, kéo xuống, thả trên item → chọn item đó (`Select/SelectItem.vue:110-113,135-137`; test `Select/Select.browser.test.ts:237-255`).
- Thả trong ≤ 10 px theo cả hai trục so với điểm nhấn trigger → không chọn gì. Popup vừa mở dưới con trỏ (`Select/SelectContentImpl.vue:173-175`).
- Kéo > 10 px rồi thả ngoài popup → đóng (`Select/SelectContentImpl.vue:176-180`).
- Chuột/bút chọn khi nhả trên item (`Select/SelectItem.vue:135-137`).
- Chạm: trigger mở khi nhấc ngón, không mở khi chạm xuống (`Select/SelectTrigger.vue:125-128,170-179`).
- Chạm: item chỉ được chọn khi lần chạm bắt đầu trên chính item đó. Cuộn bằng ngón không bao giờ chọn. Lần chạm mở popup không chọn item vừa hiện dưới ngón (`Select/SelectItem.vue:110-154`; test `Select/Select.browser.test.ts:174-235`).
- Chạm: ngưỡng 10 px không áp dụng (`Select/SelectContentImpl.vue:167-170`).
- Kích hoạt không do con trỏ (AT) → chọn (`Select/SelectItem.vue:149-153`).
- Một lần nhấn phím chọn chỉ chọn một lần (`Select/SelectItem.vue:124-126,156-161`).
- Nút cuộn chỉ hiện khi còn cuộn được theo hướng đó (`Select/SelectScrollUpButton.vue:26-38`, `Select/SelectScrollDownButton.vue:26-41`).
- Di hoặc nhấn trên nút cuộn → tự cuộn mỗi 50 ms, mỗi bước bằng chiều cao item neo. Rời nút → dừng (`Select/SelectScrollButtonImpl.vue:32-47,61-65`, `Select/SelectScrollUpButton.vue:50-55`, `Select/SelectScrollDownButton.vue:53-60`).
- Di trên nút cuộn → bỏ highlight (`Select/SelectScrollButtonImpl.vue:40-41`).
- Chuột phải trong popup: không có menu ngữ cảnh (`Select/SelectContentImpl.vue:354`).
- Value không nhận con trỏ. Nhấn vào chữ là nhấn trigger (`Select/SelectValue.vue:54`).

#### Lọc và tìm
- Không lọc. Chỉ typeahead (xem "Bàn phím").
- Item vô hiệu bị bỏ khi điều hướng và typeahead (`Collection/Collection.ts:39-54`).
- Khi đóng, các item vẫn được biết. Nhờ vậy trigger hiện nhãn ngay từ đầu, và typeahead chạy khi đóng (`Select/SelectContent.vue:37-44,62-72`).

#### Đóng
- `Escape` (chỉ lớp trên cùng); nhấn ngoài; chọn ở chế độ một; kéo > 10 px rồi thả ngoài.
- Focus rời popup không đóng (`Select/SelectContentImpl.vue:318`).
- Khi mở (mặc định): nhấn ngoài chỉ đóng, không tới phần tử bên dưới; trang không cuộn; mọi thứ ngoài popup ẩn khỏi AT (`Select/SelectContentImpl.vue:108-113,119,126`).
- Popup còn ở lại tới khi animation đóng xong (`Select/SelectContent.vue:48-60`).

#### ARIA và data
- Trigger: `role="combobox"`, `aria-expanded`, `aria-controls` chỉ khi mở, `aria-required`, `aria-autocomplete="none"`. Vô hiệu thật (`Select/SelectTrigger.vue:211-222`).
- Trigger trong `Field`: id, `aria-labelledby`, `aria-describedby`, `aria-invalid` lấy từ `Field` (`Select/SelectTrigger.vue:47-60`).
- Content: `role="listbox"` (`Select/SelectContentImpl.vue:344`). Không có `aria-multiselectable`, kể cả khi chọn nhiều. Đây là khoảng trống: S6 nên thêm và ghi vào `verdict/better.tsv`.
- Item: `role="option"`, `aria-selected`, `aria-disabled`. Tên lấy từ ItemText (`Select/SelectItem.vue:237-246`).
- Group: `role="group"`, tên từ Label (`Select/SelectGroup.vue:24-31`, `Select/SelectLabel.vue:20-26`).
- Ẩn khỏi AT: Separator, Icon, ItemIndicator, nút cuộn (`Select/SelectSeparator.vue:14-19`, `Select/SelectIcon.vue:16-22`, `Select/SelectItemIndicator.vue:28-38`, `Select/SelectScrollButtonImpl.vue:54`).
- Viewport: `role="presentation"` (`Select/SelectViewport.vue:76`).
- Look cần đọc: mở/đóng (cả lúc đang đóng, cho animation); trigger rỗng (hiện placeholder); vô hiệu (gốc, item); item highlight = item có focus (`shadcn-style:style-vega.css:979-980`); item được chọn (indicator); còn cuộn lên/xuống được; chế độ căn theo item (shadcn tắt animation); popper: cạnh, căn, bề rộng và cao trigger, chiều cao khả dụng, gốc biến đổi (`Select/SelectPopperPosition.vue:26-33`).

#### Form
- Chỉ tham gia khi nằm trong form và có tên, tên riêng hoặc từ `Field` (`Select/SelectRoot.vue:104,242-243`, `shared/useFormControl.ts`).
- Gửi giá trị đã chọn dưới tên đó (test `{ test: 'Banana' }`, `Select/Select.test.ts:759-778`).
- Giá trị rỗng → gửi một giá trị thay thế, mặc định `''` (`Select/SelectRoot.vue:78-82,254-257`; test `Select/Select.test.ts:747-757`).
- Mỗi lần giá trị đổi, form nhận một sự kiện change (`Select/BubbleSelect.vue:22-35`).
- Autofill của trình duyệt ghi được giá trị (`Select/BubbleSelect.vue:41-43`).
- Bắt buộc, vô hiệu, gợi ý autocomplete được truyền cho form (`Select/SelectRoot.vue:247-251`).
- `Field`: tên, bắt buộc, vô hiệu lấy từ `Field` khi Select không tự đặt (`Select/SelectRoot.vue:102-110`).
- Chọn nhiều: cách gửi nhiều giá trị chưa kiểm (không có test).

#### RTL
- Căn theo item dùng mép phải (`Select/SelectItemAlignedPosition.vue:80-95`).
- Hướng truyền cho trigger và popup (`Select/SelectTrigger.vue:221`, `Select/SelectContentImpl.vue:346`).
- Phím không đổi: danh sách dọc.

#### Cơ chế DOM không port
- Item render vào một `DocumentFragment` khi đóng (`Select/SelectContent.vue:37-44,62-72`) → trigger có nhãn, typeahead khi đóng.
- `<select>` gốc ẩn, gán value qua prototype rồi phát `change` (`Select/BubbleSelect.vue:22-72`) → gửi form, autofill.
- `key` của `<select>` đổi theo danh sách option (`Select/SelectRoot.vue:152-161`) → giá trị đầu đúng trong form.
- `preventDefault` ở `mousedown`, `releasePointerCapture` ở `pointerdown` (`Select/SelectTrigger.vue:130-153`) → trigger không giật focus; kéo–thả chọn được.
- Click label không qua `pointerdown` trên Safari (`Select/SelectTrigger.vue:155-168`) → nhấn label focus trigger.
- Chạm chọn ở `click`, không ở `pointerup` (iOS) (`Select/SelectItem.vue:110-154`) → chạm chọn đúng một lần; cuộn không chọn.
- `setTimeout(focusFirst)` cho mũi tên (`Select/SelectContentImpl.vue:233`) → mũi tên chuyển item.
- Wrapper `position: fixed`, chép `z-index`, đo `getComputedStyle` (`Select/SelectItemAlignedPosition.vue:101-130,198-205,230-238`) → căn theo item chính xác.
- CSS ẩn thanh cuộn (`Select/SelectViewport.vue:90-98`) → look: không thanh cuộn, dùng nút cuộn.
- Focus guards, `aria-hidden` cho phần còn lại (`Select/SelectContentImpl.vue:118,126`, `shared/useHideOthers.ts`) → AT chỉ thấy popup.
- Khóa cuộn body, chặn pointer ngoài (`Select/SelectContentImpl.vue:119`, `shared/bodyPointerEvents.ts`) → trang đứng yên; nhấn ngoài chỉ đóng.
- `event.detail === 0` (`Select/SelectItem.vue:149-153`) → AT kích hoạt được item.
- Cờ "phím chọn đang giữ" (`Select/SelectItem.vue:124-126,156-166`) → một phím, một lần chọn.

#### Dễ sót
- Typeahead trên trigger đóng chọn luôn, không mở. Chỉ chế độ một (`Select/SelectTrigger.vue:87-93`).
- Ngưỡng 10 px sau khi nhấn mở. Kéo rồi thả ngoài thì đóng (`Select/SelectContentImpl.vue:151-199`).
- Chạm: chỉ lần chạm bắt đầu trên item mới chọn (`Select/SelectItem.vue:118-154`).
- Item neo khác nhau giữa chế độ một và nhiều (`Select/SelectContentImpl.vue:254-280`).
- Hai luật placeholder khác nhau. Trigger "rỗng" theo giá trị. Value hiện placeholder khi không item nào khớp giá trị. Giá trị lạ → trigger không rỗng mà Value vẫn hiện placeholder (`Select/SelectTrigger.vue:224`, `Select/SelectValue.vue:31-46,55`; test `Select/Select.test.ts:644-731`).
- Placeholder không chớp khi mở/đóng (test `Select/Select.browser.test.ts:337-366`).
- Nhãn chọn nhiều = chữ các item nối bằng `", "`, theo thứ tự trong giá trị (`Select/SelectValue.vue:35-46`).
- `Space` giữa lúc gõ typeahead không chọn, không mở (trigger và item).
- `Tab` bị chặn trong popup. Focus rời popup không đóng.
- Docs ≠ nguồn: `Enter` trên trigger focus item neo, không phải "item đầu"; giống `Space` (`docs:components/select.md:664-665`).
- Docs ≠ nguồn: trạng thái placeholder của trigger theo giá trị rỗng, không theo "có placeholder" (`docs:components/select.md:122`).
- Docs ≠ nguồn: Value hiện placeholder khi không item nào khớp, không chỉ khi "không có giá trị" (`docs:components/select.md:129`, `Select/SelectValue.vue:7`).

---

### Combobox
`Combobox/` · dùng bởi: shadcn-vue `combobox` (`shadcn:combobox/`). Dựng trên `Listbox` (`Combobox/ComboboxRoot.vue:315-333`).

#### Part
- Gốc: giữ giá trị, mở/đóng, chuỗi lọc.
- Anchor: mốc đặt vị trí. Bọc input và trigger.
- Input: ô gõ. Giữ DOM focus suốt.
- Trigger: nút mở/đóng. Không nằm trong thứ tự Tab.
- Cancel: xóa chữ.
- Content (inline hoặc popper), Viewport, Item, ItemIndicator, Group, Label, Separator, Empty, Arrow (popper), Virtualizer.

#### Trạng thái
- **Giá trị**: một giá trị, hoặc danh sách khi chọn nhiều. Mặc định: không có, hoặc `[]` (`Combobox/ComboboxRoot.vue:142-146`).
  - Người dùng chọn item, chế độ một: gán giá trị, đóng. Chọn lại item đang chọn vẫn giữ nó; không bỏ chọn (`Combobox/ComboboxItem.vue:101-105`).
  - Người dùng chọn item, chế độ nhiều: bật/tắt thành viên, thêm vào cuối. Vẫn mở. Focus về input (`Combobox/ComboboxItem.vue:106-108`, `Listbox/ListboxRoot.vue:157-163`).
  - Caller có thể từ chối từng lần chọn (`Combobox/ComboboxItem.vue:96-99`).
  - Cancel chỉ xóa giá trị khi bật tùy chọn đó (mặc định tắt). Giá trị xóa = `null`, hoặc `[]` khi chọn nhiều. Caller có thể cho giá trị xóa riêng; chế độ nhiều luôn ép về danh sách (`Combobox/ComboboxCancel.vue:25-27`, `Combobox/ComboboxRoot.vue:256-262`).
  - Caller ghi giá trị khi popup đóng → chữ trong input reset theo giá trị mới, nếu bật "reset khi chọn". Lúc mở: chữ giữ nguyên (`Combobox/ComboboxInput.vue:155-158`).
  - Item không được có giá trị `''` (`Combobox/ComboboxItem.vue:42-46`).
- **Chữ trong input**: người dùng gõ. Được reset về chữ của giá trị (`Combobox/ComboboxInput.vue:129-149`):
  - Caller cho hàm hiển thị → dùng hàm đó (cả khi chọn nhiều).
  - Chế độ một, giá trị nguyên thủy → chuỗi của nó. Giá trị object → `''`.
  - Chế độ nhiều → `''`.
  - Reset khi đóng, sau 1 ms, nếu bật "reset khi đóng" (mặc định bật) (`Combobox/ComboboxRoot.vue:170-176`).
  - Reset khi giá trị đổi lúc popup đóng, nếu bật "reset khi chọn" (mặc định bật) (`Combobox/ComboboxInput.vue:155-158`).
- **Chuỗi lọc**: nội bộ, lấy từ chữ gõ. Xóa mỗi lần mở và mỗi lần đóng (`Combobox/ComboboxRoot.vue:159`).
- **Mở/đóng**: người dùng và caller.
- **Highlight**: nội bộ (của `Listbox`). Bị bỏ khi đóng (`Combobox/ComboboxRoot.vue:189-199`). Caller mở → highlight lại item chọn, hoặc item đầu (`:200-203`).
- Tùy chọn hành vi, mặc định (`Combobox/ComboboxRoot.vue:116-125`, `Combobox/ComboboxInput.vue:31-34`):
  - Reset chữ khi đóng: bật. Reset chữ khi chọn: bật.
  - Mở khi focus input: tắt. Mở khi click input: tắt.
  - Cancel xóa cả giá trị: tắt.
  - Hover highlight: bật.
  - Gỡ item khỏi cây khi đóng: bật.
  - Focus input khi mở: bật.
  - Tắt lọc có sẵn (caller tự lọc): tắt. Vòng phím: tắt.

#### Vị trí
- Inline (mặc định reka): popup nằm trong luồng. Không đặt vị trí (`Combobox/ComboboxContentImpl.vue:42-45,68-76,181-182`).
- Popper: như popover. Mặc định Popper: cạnh dưới, căn giữa, lệch 0, cách mép 0 (`Popper/PopperContent.vue:15-33`).
- shadcn: popper, căn giữa, lệch 4 px, rộng bằng anchor (`shadcn:combobox/ComboboxList.vue:12-16,28`).
- Popper: việc cuộn item highlight vào tầm nhìn đợi tới khi popup đặt xong (`Combobox/useComboboxContentPositioning.ts:11-39`, `Listbox/ListboxRoot.vue:202-221`).

#### Bàn phím
Input:

| Phím | Kết quả |
|---|---|
| `ArrowDown` / `ArrowUp` khi đóng | Mở. Chặn mặc định. Khi mở xong: highlight item chọn, hoặc item đầu (`Combobox/ComboboxInput.vue:59-66`, `Combobox/ComboboxRoot.vue:161-169`). |
| `ArrowDown` / `ArrowUp` khi mở | Item bật kế / trước. Dừng ở hai đầu, trừ khi vòng (`Listbox/ListboxFilter.vue:82-88`, `Listbox/ListboxRoot.vue:353-391`; test `Combobox/Combobox.test.ts:2066-2087`). |
| `Home` / `End` | Item đầu / cuối. Con trỏ chữ không chạy, kể cả khi đóng (`Listbox/ListboxFilter.vue:82-88,109`; test `Combobox/Combobox.test.ts:2071`). |
| `Enter` có highlight | Chọn item đó. Chặn mặc định, không lan lên: form không gửi (`Listbox/ListboxRoot.vue:240-254`; test `Combobox/Combobox.test.ts:1186-1261`). |
| `Ctrl`/`Meta`/`Alt` + `Enter` | Không xử lý. Lan lên, form có thể gửi (`Listbox/ListboxRoot.vue:244-245`). |
| `Enter` không highlight (gồm lúc đóng) | Không xử lý (`Listbox/ListboxRoot.vue:241`). |
| `Escape` | Đóng (`Combobox/ComboboxContentImpl.vue:163-167`). Chữ reset sau 1 ms nếu bật "reset khi đóng". |
| Gõ chữ, khi đóng | Mở, lọc theo chữ, highlight item khớp đầu (`Combobox/ComboboxInput.vue:68-81`). |
| Gõ chữ, khi mở | Lọc. Mỗi lần gõ highlight item khớp đầu (`Combobox/ComboboxInput.vue:78-80`, `Listbox/ListboxFilter.vue:75-80`). |
| Đang soạn IME | Mũi tên và `Enter` thuộc IME. Không lọc, không chọn (`Combobox/ComboboxInput.vue:60-62`, `Listbox/ListboxFilter.vue:84-85,92-93`). |
| `Tab` | Không xử lý. Focus rời → đóng (xem "Đóng"). |
| `PageUp` / `PageDown`, `Space`, trái/phải | Không xử lý. Trái/phải chạy con trỏ chữ. |

Trigger:

| Phím | Kết quả |
|---|---|
| Click | Bật/tắt mở (`Combobox/ComboboxTrigger.vue:53`). |
| `ArrowDown` / `ArrowUp` | Chỉ khi mở và tắt "focus input khi mở": chuyển focus vào input (`Combobox/ComboboxTrigger.vue:30-36`). |

#### Focus
- DOM focus ở input. Item highlight báo qua `aria-activedescendant` (`Combobox/ComboboxInput.vue:179`).
- Item không nhận focus (`Listbox/ListboxFilter.vue:50-51`, `Listbox/ListboxItem.vue:95`).
- Mở → focus input (mặc định) (`Combobox/ComboboxRoot.vue:167-168`).
- Input nằm trong popup → focus khi popup hiện (`Combobox/ComboboxContentImpl.vue:96-115`).
- Đóng khi focus đang trong popup → focus về trigger (`Combobox/ComboboxContentImpl.vue:118-126`).
- Popup bị gỡ mà focus rơi mất → focus về trigger (`Combobox/ComboboxContentImpl.vue:129-135`).
- Chọn ở chế độ nhiều → focus về input (`Combobox/ComboboxItem.vue:106-108`).
- Cancel → xóa chữ, focus input (`Combobox/ComboboxCancel.vue:20-23`).
- Không bẫy focus. Không tự chuyển focus khi popup mount/unmount (`Combobox/ComboboxContentImpl.vue:157-162`).

#### Pointer
- Hover → highlight (mặc định bật). Không cuộn, không focus (`Listbox/ListboxItem.vue:106-112`).
- Click item → chọn (`Listbox/ListboxItem.vue:104`).
- Nhấn ngoài → đóng. Trừ nhấn trong gốc, hoặc vào label gắn với một điều khiển trong gốc (`Combobox/ComboboxContentImpl.vue:137-148,175-179`).
- Mặc định không modal: nhấn ngoài vẫn tới phần tử bên dưới; trang vẫn cuộn (`Combobox/ComboboxContentImpl.vue:62-63,166`, `DismissableLayer/DismissableLayer.vue:82`).
- Mở khi click / khi focus input: tùy chọn, mặc định tắt (`Combobox/ComboboxInput.vue:89-92,124-127`).

#### Lọc và tìm
- Lọc có sẵn: item hiện khi chữ của nó chứa chuỗi gõ. Không phân biệt hoa thường và dấu (`Combobox/ComboboxRoot.vue:209-254`, `shared/useFilter.ts:19-65`).
- Chữ của item = chữ caller cho, không có thì chữ hiển thị. Ghi một lần lúc mount (`Combobox/ComboboxItem.vue:65-67`).
- Item không khớp bị gỡ khỏi cây, không chỉ ẩn (`Combobox/ComboboxItem.vue:48-63,89`).
- Group không còn item hiện → ẩn (`Combobox/ComboboxGroup.vue:25,46`).
- Chuỗi rỗng, tắt lọc, hoặc danh sách ảo → không lọc (`Combobox/ComboboxRoot.vue:218`).
- Số kết quả từ 0 lên ≥ 1 → highlight item đầu (trừ danh sách ảo) (`Combobox/ComboboxInput.vue:160-166`).
- Không khớp gì → hiện Empty. Chữ mặc định "No options" (`Combobox/ComboboxEmpty.vue:15-29`).
- Tùy chọn ẩn cả popup khi rỗng: mặc định tắt (`Combobox/ComboboxContentImpl.vue:20-24,195`).
- IME: không lọc từ chuỗi soạn dở. Ngoại lệ Android, chữ Latin đang soạn → lọc ngay (`shared/useComposing.ts:34,42-59`).
- Không có typeahead: focus ở input.

#### Đóng
- `Escape`; nhấn ngoài (trừ trong gốc và label gắn); chọn ở chế độ một; bấm lại trigger.
- Focus rời cả gốc lẫn popup → đóng. Kiểm lại sau 1 frame; nếu focus đã quay vào thì không đóng (`Combobox/ComboboxInput.vue:94-122`, `Combobox/ComboboxContentImpl.vue:168-172`).
- Focus sang chỗ không nhận focus → để lớp dismiss xử lý (`Combobox/ComboboxInput.vue:100-102`).
- Khi đóng: highlight bỏ; chuỗi lọc xóa; chữ reset (theo tùy chọn).

#### ARIA và data
- Input: `role="combobox"`, `aria-expanded`, `aria-controls` và `aria-activedescendant` chỉ khi mở, `aria-autocomplete="list"`. Tắt autocomplete của trình duyệt (`Combobox/ComboboxInput.vue:170-182`).
- Trigger: `tabindex="-1"`, tên "Show popup", `aria-haspopup="listbox"`, `aria-expanded`, `aria-controls` khi mở, `aria-disabled` (`Combobox/ComboboxTrigger.vue:39-55`).
- Popup: `role="listbox"`, `aria-orientation`, `aria-multiselectable` (`Listbox/ListboxContent.vue:24-29`).
- Item: `role="option"`, `aria-selected`, `aria-disabled` (`Listbox/ListboxItem.vue:94-97`).
- Empty: `role="option"`, `aria-disabled="true"` (`Combobox/ComboboxEmpty.vue:22-28`).
- Group: `role="group"`, tên từ Label (`Combobox/ComboboxGroup.vue:42-47`, `Combobox/ComboboxLabel.vue:21-30`).
- Khi mở: mọi thứ ngoài gốc bị ẩn khỏi AT (`Combobox/ComboboxContentImpl.vue:65-66`). Popup nằm trong portal có bị ẩn theo không: chưa kiểm (thư viện `aria-hidden` không có trong `references/`).
- Look cần đọc: mở/đóng; item highlight (`shadcn-style:style-vega.css:300-302`); item được chọn (indicator); vô hiệu; danh sách rỗng, để hiện Empty (`shadcn-style:style-vega.css:308-310`); popper: cạnh, căn, bề rộng anchor, chiều cao khả dụng, gốc biến đổi (`Combobox/ComboboxContentImpl.vue:80-91`).

#### Form
- Qua `Listbox`: input ẩn khi trong form và có tên (`Listbox/ListboxRoot.vue:545-551`). Luật đặt tên: xem "Ghi chú chung".
- `Enter` chọn item không gửi form.

#### RTL
- Hướng truyền cho `Listbox` (`Combobox/ComboboxRoot.vue:324`). Danh sách dọc: trái/phải không điều hướng (`RovingFocus/utils.ts:38-39`).

#### Cơ chế DOM không port
- `requestAnimationFrame` trước khi đóng do blur (`Combobox/ComboboxInput.vue:108-121`) → chuyển giữa hai combobox không đóng nhầm.
- `setTimeout(1)` cho reset chữ (`Combobox/ComboboxRoot.vue:172-175`) → chữ reset sau khi chọn/đóng xong.
- Gán lại chữ sau `nextTick` (`Combobox/ComboboxInput.vue:144-148`) → input luôn hiện đúng chữ.
- `label.control` (`Combobox/ComboboxContentImpl.vue:141-147`) → nhấn label không đóng rồi mở lại.
- `v-memo` (`Combobox/ComboboxItem.vue:94`) → hiệu năng.
- Thuộc tính `hidden` cho group (`Combobox/ComboboxGroup.vue:46`) → group rỗng ẩn.
- `display: none` khi rỗng (`Combobox/ComboboxContentImpl.vue:195`) → tùy chọn ẩn popup.
- Hoãn cuộn highlight tới khi popper đặt xong (`Combobox/useComboboxContentPositioning.ts`) → trang không nhảy.
- `useHideOthers` (`Combobox/ComboboxContentImpl.vue:65-66`) → AT chỉ thấy combobox.
- `pointer-events: auto` trên gốc khi mở (`Combobox/ComboboxRoot.vue:319-321`) → gốc vẫn nhận con trỏ khi body bị khóa.

#### Dễ sót
- `Enter` khi đóng không bao giờ chọn: highlight bị bỏ khi đóng, kể cả khi item còn trong cây (`Combobox/ComboboxRoot.vue:189-199`; test `Combobox/Combobox.test.ts:149-293`).
- Chế độ một: chọn lại item đang chọn không bỏ chọn. Khác `Listbox` (`Combobox/ComboboxItem.vue:101-105`, `Listbox/ListboxRoot.vue:170-175`).
- Muốn giữ chữ gõ sau khi chọn (chế độ một): phải tắt cả hai tùy chọn reset (`Combobox/ComboboxInput.vue:155-158`, `Combobox/ComboboxRoot.vue:170-176`).
- Mở lại: chuỗi lọc rỗng, nên mọi item hiện dù input còn chữ (`Combobox/ComboboxRoot.vue:159`).
- `Home`/`End` trong input điều hướng danh sách, không chạy con trỏ chữ.
- `Ctrl`/`Meta`/`Alt` + `Enter` lan lên form.
- Docs ≠ nguồn: bảng phím nói focus nằm trên item; nguồn giữ focus ở input với `aria-activedescendant`. Bảng thiếu `Home`/`End`. `Escape` chỉ khôi phục chữ khi bật "reset khi đóng" (`docs:components/combobox.md:727-745`).
- Docs ≠ nguồn: Cancel còn focus input và có thể xóa giá trị (`docs:components/combobox.md:137`).
- Docs ≠ nguồn: hàm hiển thị ghi "không dùng với chọn nhiều", nhưng vẫn được áp dụng (`Combobox/ComboboxInput.vue:10,132-133`).

---

### Listbox
`Listbox/` · dùng bởi: shadcn-vue `command` trực tiếp (`shadcn:command/Command.vue:81-87`, `shadcn:command/CommandList.vue:16-24`); `combobox` gián tiếp qua `Combobox`.

#### Part
- Gốc: giá trị, highlight.
- Content: danh sách.
- Item, ItemIndicator, Group, GroupLabel.
- Filter: ô tìm, tùy chọn. Khi có, focus ở ô này.
- Virtualizer: danh sách ảo.

#### Trạng thái
- **Giá trị**: một giá trị, hoặc danh sách khi chọn nhiều. Mặc định: không có, hoặc `[]` (`Listbox/ListboxRoot.vue:149-153`).
- **Cách chọn**: bật/tắt (mặc định) hoặc thay (`Listbox/ListboxRoot.vue:117-120,155-179`):
  - Một + bật/tắt: chọn lại item đang chọn → bỏ chọn (test `Listbox/Listbox.test.ts:75-81`).
  - Một + thay: luôn gán.
  - Nhiều + bật/tắt: thêm vào cuối, hoặc bỏ.
  - Nhiều + thay: giá trị = [item]. Item đó thành mốc cho `Shift` (`:164-166`).
- Người dùng ghi giá trị: click, `Enter`, `Space`; `Ctrl`/`Alt`/`Meta`+`A` (nhiều); `Shift` + điều hướng (nhiều + thay).
- Caller có thể từ chối từng lần chọn (`Listbox/ListboxItem.vue:66-75`).
- Caller ghi giá trị (`Listbox/ListboxRoot.vue:466-483`):
  - Lần đầu (mount): chỉ highlight item chọn, hoặc item bật đầu. Không focus, không cuộn trang (`:444-452`).
  - Lần sau: highlight, cuộn vào tầm nhìn, và chuyển focus tới item đó.
  - Không chuyển focus khi có Filter (`:203`, `Listbox/ListboxFilter.vue:51`).
  - Bật "giữ focus" và focus đang ngoài listbox: không focus, không cuộn (`:475`).
- **Highlight**: nội bộ. Người dùng ghi bằng phím, hover (tùy chọn, mặc định tắt), chọn. Caller có thể đặt highlight theo giá trị, có hoặc không chuyển focus (`Listbox/ListboxRoot.vue:227-238`).
- Tùy chọn: hướng dọc (mặc định) hoặc ngang; vòng: tắt; hover highlight: tắt.
- shadcn `command`: giá trị cố định `''` trừ khi caller tự buộc. Chọn chỉ là hành động, không lưu. Hover highlight bật (`shadcn:command/Command.vue:10-13`).

#### Bàn phím
Content (không Filter, focus trên item):

| Phím | Kết quả |
|---|---|
| `ArrowDown` / `ArrowUp` (dọc); `ArrowRight` / `ArrowLeft` (ngang, RTL đảo) | Item bật kế / trước. Chưa highlight: tiến → item đầu, lùi → item cuối. Dừng ở hai đầu, trừ khi vòng (`Listbox/ListboxRoot.vue:353-391`, `RovingFocus/utils.ts:9-43`). Phím trục kia bị bỏ (`Listbox/ListboxContent.vue:37-51`). |
| `Home` / `End` | Item đầu / cuối. Chưa highlight: `End` → item đầu (lỗi) (`Listbox/ListboxRoot.vue:359-362,384-386`). |
| `PageUp` / `PageDown` | Chỉ danh sách ảo: đầu / cuối (`Listbox/ListboxVirtualizer.vue:300,313-324`). Không ảo: không xử lý (`Listbox/ListboxContent.vue:37`). |
| `Enter` | Chọn item highlight. Chặn mặc định, không lan lên. `Ctrl`/`Meta`/`Alt` + `Enter` lan lên (`Listbox/ListboxRoot.vue:240-254`). |
| `Space` | Chọn item đang focus. Chặn cuộn trang (`Listbox/ListboxItem.vue:105`). |
| `Ctrl`/`Alt`/`Meta` + `A` | Chọn nhiều: chọn mọi item bật, highlight item cuối (`Listbox/ListboxRoot.vue:266-274`). Ảo: mọi option, kể cả vô hiệu (`Listbox/ListboxVirtualizer.vue:303-308`). |
| `Shift` + điều hướng | Nhiều + thay: chọn dải từ mốc tới item mới. `Shift`+`Home`/`End`: tới đầu/cuối (`Listbox/ListboxRoot.vue:393-415`). |
| Ký tự in được | Typeahead: highlight và focus item khớp, 1000 ms (`Listbox/ListboxRoot.vue:275-279`). |
| Ký tự, danh sách ảo | So chữ caller cho, hoặc chuỗi của option viết thường. Tìm trong toàn bộ option, không chỉ hàng đang vẽ (`Listbox/ListboxVirtualizer.vue:250-264,325-341`). |

Filter (focus ở ô tìm):

| Phím | Kết quả |
|---|---|
| `ArrowDown` / `ArrowUp` / `Home` / `End` | Điều hướng như trên. Chặn mặc định: `Home`/`End` không chạy con trỏ chữ. Bỏ qua khi soạn IME (`Listbox/ListboxFilter.vue:82-88,109`). |
| `Enter` | Chọn item highlight. Bỏ qua khi soạn IME (`Listbox/ListboxFilter.vue:90-95`). |
| Gõ chữ | Cập nhật chuỗi tìm. Highlight item đầu (`Listbox/ListboxFilter.vue:75-80`). |
| `Space`, `Ctrl`+`A`, ký tự | Thuộc ô chữ. Không chọn, không typeahead (`Listbox/ListboxRoot.vue:256-258`). |

#### Focus
- Không Filter: roving, một điểm Tab. Item highlight có `tabindex="0"`. Chưa highlight → chính danh sách nhận Tab (`Listbox/ListboxItem.vue:95`, `Listbox/ListboxContent.vue:27`).
- Vào danh sách bằng Tab: highlight item trước đó, hoặc item bật đầu (`Listbox/ListboxRoot.vue:323-338`). Caller có thể chặn.
- Vào bằng nhấn chuột: không làm vậy; cửa sổ 10 ms (`Listbox/ListboxContent.vue:18,31-36`).
- Focus rời gốc → bỏ highlight, nhớ item để vào lại (`Listbox/ListboxRoot.vue:302-321`).
- Có Filter: item không nhận focus. Ô tìm giữ focus. Highlight báo qua `aria-activedescendant` (`Listbox/ListboxFilter.vue:47-51,107`).
- Ô tìm tự focus khi mount nếu bật, sau 1 ms (`Listbox/ListboxFilter.vue:53-57`). shadcn bật (`shadcn:command/CommandInput.vue:32-36`).
- Item đang highlight bị gỡ → highlight được nhả (`Listbox/ListboxItem.vue:59-64`).

#### Pointer
- Click → chọn (`Listbox/ListboxItem.vue:104`).
- Hover highlight (khi bật): không cuộn, không focus (`Listbox/ListboxItem.vue:106-112`).
- Con trỏ rời gốc → bỏ highlight (`Listbox/ListboxRoot.vue:540`).

#### Lọc và tìm
- `Listbox` không tự lọc. Filter chỉ giữ chuỗi tìm (`Listbox/ListboxFilter.vue:37-40`).
- shadcn `command` tự lọc: "chứa chuỗi", không phân biệt hoa thường và dấu (`shadcn:command/Command.vue:24,37-67`).
- Chữ để lọc của item command = chữ hiển thị lúc mount, hoặc chuỗi của giá trị (`shadcn:command/CommandItem.vue:46`).
- Item command không khớp bị gỡ (`shadcn:command/CommandItem.vue:22-37,65`). Group rỗng ẩn (`shadcn:command/CommandGroup.vue:20,38`).
- Empty command hiện khi có chuỗi tìm và không khớp gì (`shadcn:command/CommandEmpty.vue:15`).
- Chọn item command → xóa chuỗi tìm (`shadcn:command/CommandItem.vue:71-73`).
- Không Filter: typeahead (xem bảng).

#### Ảo hóa
- Ước lượng 28 px mỗi hàng. Vẽ dư 12 hàng. Caller có thể đổi (`Listbox/ListboxVirtualizer.vue:71-78`).
- Hàng highlight và hai hàng kề luôn được vẽ, kể cả khi cuộn xa (`Listbox/ListboxVirtualizer.vue:79-96`).
- Highlight thuộc về option, không về hàng. Option đổi chỗ → highlight đi theo; focus theo nếu hàng cũ có focus; không cuộn. Option bị bỏ → highlight được nhả (`Listbox/ListboxVirtualizer.vue:100-148`).
- Mỗi hàng báo `aria-setsize` = tổng số option, `aria-posinset` = chỉ số + 1 (`Listbox/ListboxVirtualizer.vue:161-167`).
- Vòng chỉ xảy ra ở đầu/cuối thật của toàn bộ option (`Listbox/ListboxRoot.vue:340-351,368-376`).
- Caller ghi giá trị: cuộn tới option chọn (đặt ở đầu khung), rồi highlight (`Listbox/ListboxVirtualizer.vue:184-206`).
- Mount: không focus, không cuộn trang (`Listbox/ListboxVirtualizer.vue:179-183,220-233`).
- Padding đầu/cuối của khung cuộn được tính khi cuộn tới (`Listbox/ListboxVirtualizer.vue:47-59,67-68`).
- Test: `Listbox/ListboxVirtualizer.browser.test.ts:58-160`.

#### Đóng
- Không có popup. `CommandDialog` đóng qua `Dialog` (`shadcn:command/CommandDialog.vue:25-37`; nhóm khác).

#### ARIA và data
- Content: `role="listbox"`, `aria-orientation`, `aria-multiselectable` (`Listbox/ListboxContent.vue:24-29`).
- Item: `role="option"`, `aria-selected`, `aria-disabled` (`Listbox/ListboxItem.vue:94-97`).
- Group: `role="group"`, tên từ GroupLabel (`Listbox/ListboxGroup.vue:24-31`, `Listbox/ListboxGroupLabel.vue:20-26`).
- Filter: `aria-activedescendant` = item highlight; `aria-disabled` (`Listbox/ListboxFilter.vue:99-115`).
- Filter không có `role="combobox"`, `aria-controls`, `aria-expanded`. Khoảng trống: AT không biết ô tìm điều khiển danh sách nào. S6 nên gắn quan hệ đó và ghi vào `verdict/better.tsv`.
- Look cần đọc: item highlight (`shadcn-style:style-vega.css:385-386`); item được chọn; item vô hiệu; hướng; group ẩn; không khớp gì (Empty).
- Look, chỗ Chrome sai: shadcn `command` đọc trạng thái vô hiệu và được chọn theo dạng reka không phát. Trong Chrome, item command vô hiệu không mờ, dấu check không hiện (`shadcn:command/CommandItem.vue:70,82`, `Listbox/ListboxItem.vue:100-103`). S6 chọn khớp Chrome hay sửa, và ghi vào `verdict/better.tsv`.

#### Form
- Input ẩn khi trong form và có tên (`Listbox/ListboxRoot.vue:144,545-551`). Luật đặt tên: xem "Ghi chú chung".

#### RTL
- Hướng ngang: trái/phải đảo theo RTL (`RovingFocus/utils.ts:20-28`).

#### Cơ chế DOM không port
- `mousedown` + cờ tự tắt sau 10 ms (`Listbox/ListboxContent.vue:18,31-36`) → nhấn chuột không bị coi là vào bằng Tab.
- Sự kiện `PageUp` giả (`Listbox/ListboxRoot.vue:295-300`) → highlight item đầu.
- Cờ "hành động người dùng" tắt sau 1 ms (`Listbox/ListboxRoot.vue:180-182,281-283`) → tách ghi của người dùng khỏi ghi của caller.
- `highlightedElement.click()` khi `Enter` (`Listbox/ListboxRoot.vue:251`) → `Enter` = chọn.
- TanStack virtualizer, `translateY`, `data-index`, `cloneVNode` (`Listbox/ListboxVirtualizer.vue:65-177`) → danh sách ảo.
- Bộ đếm thế hệ cho "giữ focus" (`Listbox/ListboxRoot.vue:462-481`) → một lần ghi cũ không chạy sau lần ghi mới.
- `v-memo` (`Listbox/ListboxItem.vue:93`) → hiệu năng.

#### Dễ sót
- Caller ghi giá trị → focus nhảy vào item chọn (không Filter, không "giữ focus"). Mount thì không (`Listbox/ListboxRoot.vue:444-452,466-483`).
- Chế độ một + bật/tắt: chọn lại là bỏ chọn, kể cả bằng `Enter` (`Listbox/ListboxRoot.vue:170-175`).
- `End` khi chưa highlight → item đầu. Lỗi; S6 nên sửa và ghi vào `verdict/better.tsv`.
- Chọn tất cả không nhất quán: không ảo chỉ item bật; ảo gồm cả vô hiệu (`Listbox/ListboxRoot.vue:266-269`, `Listbox/ListboxVirtualizer.vue:303-305`).
- Hàng highlight không bị gỡ khi cuộn xa (ảo).
- Docs ≠ nguồn: bảng phím thiếu `Space` và `PageUp`/`PageDown` (ảo). Chọn tất cả còn nhận `Alt`, chỉ ở chế độ nhiều, và (không ảo) chỉ item bật (`docs:components/listbox.md:383-411`).

---

### TagsInput
`TagsInput/` · dùng bởi: shadcn-vue `tags-input` (`shadcn:tags-input/`).

#### Part
- Gốc: danh sách tag, tag đang chọn, trạng thái không hợp lệ.
- Input: bản nháp.
- Item: một tag. ItemText: tên tag. ItemDelete: nút xóa tag.
- Clear: xóa hết.

#### Trạng thái
- **Danh sách tag**. Mặc định `[]` (`TagsInput/TagsInputRoot.vue:79-84,97-101`).
- Người dùng thêm tag bằng: `Enter`; ký tự phân cách; `Tab` (tùy chọn, tắt); dán (tùy chọn, tắt); rời ô (tùy chọn, tắt) (`TagsInput/TagsInputInput.vue:27-150`).
- Luật thêm (`TagsInput/TagsInputRoot.vue:142-176`):
  - Danh sách object cần hàm đổi chữ → object. Thiếu → lỗi (`:144-151`).
  - Đã đủ tối đa (0 = không giới hạn) → từ chối. Không bật trạng thái không hợp lệ. Nháp giữ nguyên (`:153-156`).
  - Trùng → từ chối, bật trạng thái không hợp lệ. Trừ khi cho phép trùng (mặc định không) (`:158-172`).
  - Trùng = so `===`: phân biệt hoa thường; object khác tham chiếu không bao giờ trùng (`:164`).
  - Không trim. Không bỏ chuỗi rỗng ở bước này.
  - Mỗi lần thêm hoặc từ chối được báo cho caller (`:153-175`).
  - Thêm được → nháp xóa (`TagsInput/TagsInputInput.vue:46-48,91-93,118-120`).
- Người dùng xóa tag bằng: `Backspace`/`Delete` trên tag đang chọn; nút xóa của tag; Clear (xóa hết, kể cả tag vô hiệu) (`TagsInput/TagsInputRoot.vue:189-205`, `TagsInput/TagsInputItemDelete.vue:24-28`, `TagsInput/TagsInputClear.vue:19-23`).
- Xóa đúng lần xuất hiện: theo chỉ số caller cho, nếu khớp; không thì theo thứ tự hiển thị trong các tag bằng nhau (`TagsInput/TagsInputRoot.vue:122-138`).
- Caller ghi: thay cả danh sách.
- **Tag đang chọn** (bằng phím): nội bộ. Bỏ khi rời ô (`TagsInput/TagsInputInput.vue:28`).
- **Không hợp lệ**: bật khi từ chối vì trùng. Tắt ở lần gõ kế (`TagsInput/TagsInputRoot.vue:171`, `TagsInput/TagsInputInput.vue:103`).
- Chữ hiển thị của tag: mặc định chuỗi của giá trị (`TagsInput/TagsInputRoot.vue:83`).
- Ký tự phân cách: `,` mặc định; một chuỗi hoặc một regex (`TagsInput/TagsInputRoot.vue:81`, `TagsInput/TagsInputInput.vue:107-108`).

#### Bàn phím
Input. Phím tag chỉ chạy khi: không soạn IME; có ít nhất một tag bật; con trỏ chữ ở vị trí 0, không bôi đen (`TagsInput/TagsInputRoot.vue:179-193,213-214`, `TagsInput/TagsInputInput.vue:124-130`).

| Phím | Kết quả |
|---|---|
| `Enter`, nháp không rỗng | Thêm. Chặn gửi form ngay, đồng bộ (`TagsInput/TagsInputInput.vue:58-98`; test `TagsInput/TagsInput.test.ts:535-547`). |
| `Enter`, nháp rỗng | Form gửi như thường (test `TagsInput/TagsInput.test.ts:549-558`). |
| `Enter` caller đã chặn | Không thêm (`TagsInput/TagsInputInput.vue:67-86`; test `TagsInput/TagsInput.test.ts:560-575`). |
| `Tab` (khi bật), nháp không rỗng | Thêm. Focus ở lại ô, kể cả khi bị từ chối (`TagsInput/TagsInputInput.vue:51-56,95-97`; test `TagsInput/TagsInput.test.ts:169-180`). |
| `Tab`, nháp rỗng | Focus đi tiếp (test `TagsInput/TagsInput.test.ts:181-190`). |
| Ký tự phân cách | Bỏ ký tự đó khỏi nháp. Nháp chỉ còn khoảng trắng → xóa nháp. Còn lại → thêm (`TagsInput/TagsInputInput.vue:100-122`). |
| `Backspace` | Chưa chọn tag → chọn tag cuối. Đã chọn → xóa nó; chọn tag kế, hoặc tag trước nếu nó là cuối (`TagsInput/TagsInputRoot.vue:189-205`). |
| `Delete` | Đã chọn → như `Backspace`. Chưa chọn → không gì. |
| `ArrowLeft` (LTR) / `ArrowRight` (RTL) | Chưa chọn → chọn tag cuối. Đã chọn → tag trước, không vòng (`TagsInput/TagsInputRoot.vue:206-236`). |
| `ArrowRight` (LTR) / `ArrowLeft` (RTL) | Đang chọn tag cuối → bỏ chọn, về gõ. Đang chọn tag khác → tag kế. |
| `Home` / `End` | Chưa chọn → chọn tag cuối (cả `Home`). Đã chọn → tag đầu / cuối (`TagsInput/TagsInputRoot.vue:206-236`, `shared/useArrowNavigation.ts:95-139`). |
| `ArrowUp` / `ArrowDown` | Đang chọn tag → chặn mặc định (`TagsInput/TagsInputRoot.vue:238-243`). |
| Phím khác | Bỏ chọn tag (`TagsInput/TagsInputRoot.vue:244-246`). |
| Dán (khi bật) | Không chèn vào nháp. Tách theo ký tự phân cách. Thêm từng đoạn (`TagsInput/TagsInputInput.vue:132-150`; test `TagsInput/TagsInput.test.ts:314-339`). |

#### Focus
- Focus luôn ở input. "Tag đang chọn" chỉ là trạng thái, báo qua `aria-current` (`TagsInput/TagsInputItem.vue:63`).
- Nút xóa tag không nằm trong thứ tự Tab (`TagsInput/TagsInputItemDelete.vue:33`). Clear là nút thường, có trong thứ tự Tab (`TagsInput/TagsInputClear.vue:27-31`).
- Tự focus khi mount nếu bật, sau 1 ms (`TagsInput/TagsInputInput.vue:152-165`).
- "Focus trong" = focus ở bất kỳ phần nào của gốc (`TagsInput/TagsInputRoot.vue:104`).

#### Pointer
- Nhấn nút xóa → xóa tag đó, trừ khi vô hiệu (`TagsInput/TagsInputItemDelete.vue:24-28`).
- Nhấn Clear → xóa hết, trừ khi gốc vô hiệu (`TagsInput/TagsInputClear.vue:19-23`).
- Rời ô (khi bật thêm-khi-rời) → thêm nháp. Trừ khi focus sang phần tử mà ô điều khiển (danh sách gợi ý): item được chọn ở đó thắng (`TagsInput/TagsInputInput.vue:35-41`).

#### Lọc và tìm
- Không có.

#### Đóng
- Không có popup.

#### ARIA và data
- Tag: tên từ ItemText; `aria-current` = đang chọn (`TagsInput/TagsInputItem.vue:62-63`). Gốc và tag không có role riêng.
- Nút xóa: tên = chữ của tag, trừ khi có tên riêng; `aria-current` (`TagsInput/TagsInputItemDelete.vue:35-36`; test `TagsInput/TagsInput.test.ts:488-505`).
- Khoảng trống: AT nghe "apple", không phải "xóa apple". S6 nên đặt tên hành động và ghi vào `verdict/better.tsv`.
- Input: tắt autocomplete, autocorrect, autocapitalize (`TagsInput/TagsInputInput.vue:173-175`).
- Look cần đọc: focus trong gốc (`shadcn:tags-input/TagsInput.vue:20`); tag đang chọn bằng phím (`shadcn:tags-input/TagsInputItem.vue:17`); vô hiệu; không hợp lệ. shadcn đọc trạng thái không hợp lệ do caller đặt, không đọc trạng thái trùng của reka (`shadcn:tags-input/TagsInput.vue:21`).

#### Form
- Input ẩn khi trong form và có tên (`TagsInput/TagsInputRoot.vue:276-282`).
- Danh sách → `name[0]`, `name[1]`, …; tag object → `name[i][khóa]`; rỗng + bắt buộc → một input rỗng bắt buộc (`VisuallyHidden/VisuallyHiddenInput.vue:15-57`).
- `Enter` với nháp không rỗng không gửi form. Nháp rỗng thì gửi.

#### RTL
- Trái/phải đảo (`TagsInput/TagsInputRoot.vue:210`).

#### Cơ chế DOM không port
- Bọc `event.preventDefault` để biết listener khác đã chặn (`TagsInput/TagsInputInput.vue:71-76`) → caller chặn `Enter` thì không thêm.
- `preventDefault` đồng bộ trước `nextTick` (`TagsInput/TagsInputInput.vue:79-82`) → chặn gửi form ngầm.
- `selectionStart` / `selectionEnd` (`TagsInput/TagsInputRoot.vue:191,213`) → phím tag chỉ chạy khi con trỏ ở đầu.
- `relatedTarget.closest('#' + aria-controls)` (`TagsInput/TagsInputInput.vue:38-41`) → chọn gợi ý thắng thêm-khi-rời.
- `event.data` của sự kiện `input` (`TagsInput/TagsInputInput.vue:104-108`) → nhận ra ký tự phân cách vừa gõ.
- `useFocusWithin` (`TagsInput/TagsInputRoot.vue:104`) → look "focus trong".

#### Dễ sót
- `Backspace` hai lần: lần 1 chọn tag cuối, lần 2 xóa (`TagsInput/TagsInputRoot.vue:194-203`).
- `Enter` với nháp chặn gửi form; nháp rỗng thì không (`TagsInput/TagsInputInput.vue:79-82`).
- Tối đa: từ chối mà không báo không hợp lệ. Trùng: báo (`TagsInput/TagsInputRoot.vue:153-156,171`).
- Dán: đoạn rỗng hoặc có khoảng trắng vẫn thành tag (`TagsInput/TagsInputInput.vue:139-145`). Gõ phân cách thì bỏ nháp trắng. Hai đường khác nhau. S6 nên thống nhất (trim, bỏ rỗng) và ghi vào `verdict/better.tsv`.
- Xóa xong chọn tag kế, không phải tag bên trái.
- `Home` khi chưa chọn → tag cuối.
- Docs ≠ nguồn: `Backspace` "chọn tag bên trái"; nguồn chọn tag kế, trừ khi vừa xóa tag cuối (`docs:components/tags-input.md:209-211`).
- Docs ≠ nguồn: `Home` "chọn tag đầu"; nguồn: chưa chọn → tag cuối (`docs:components/tags-input.md:221-223`).
- Docs ≠ nguồn: "focused" khi focus ở input; nguồn: ở bất kỳ phần nào của gốc (`docs:components/tags-input.md:70-71`).
- Docs ≠ nguồn: "invalid" khi giá trị không hợp lệ; nguồn: chỉ khi từ chối vì trùng (`docs:components/tags-input.md:74-75,133-134`).

---

### Autocomplete
`Autocomplete/` · dùng bởi: không có trong shadcn-vue. Dùng lại các part của `Combobox` (cùng ngữ cảnh) trên `Listbox` (`Autocomplete/AutocompleteRoot.vue:218-248,256-281`). Chỉ ghi chỗ khác `Combobox`.

#### Part
- Như `Combobox`.

#### Trạng thái
- **Giá trị = chữ trong ô**. Mặc định `''` (`Autocomplete/AutocompleteRoot.vue:98-101`).
- Người dùng gõ → ghi giá trị ngay (`Autocomplete/AutocompleteInput.vue:57-72`).
- Người dùng chọn item → giá trị = chuỗi của giá trị item, không phải chữ hiển thị. `null` → `''` (`Autocomplete/AutocompleteRoot.vue:111-121`).
- Caller ghi → ô hiện chữ đó (`Autocomplete/AutocompleteInput.vue:90-95`).
- Không có chế độ nhiều. Luôn focus ô khi mở. Không reset khi chọn. Cancel luôn xóa giá trị về `''`. Luôn gỡ item khi đóng (`Autocomplete/AutocompleteRoot.vue:219-248`).
- "Reset khi đóng": mặc định tắt. Bật cũng không đổi chữ, vì chữ chính là giá trị (`Autocomplete/AutocompleteRoot.vue:75-81,138-141`, `Autocomplete/AutocompleteInput.vue:97-100`; test `Autocomplete/Autocomplete.test.ts:60-81`).

#### Vị trí
- Như `Combobox`.

#### Bàn phím
- Như `Combobox` (input).
- Gõ luôn highlight item khớp đầu. `Enter` khi đó thay chữ gõ bằng item ấy (`Listbox/ListboxFilter.vue:75-80`, `Listbox/ListboxRoot.vue:240-254`).
- Không còn khớp → không highlight; `aria-activedescendant` bị gỡ (test `Autocomplete/Autocomplete.test.ts:91-98`). `Enter` khi đó không bị chặn.

#### Focus
- Như `Combobox`.

#### Pointer
- Như `Combobox`.

#### Lọc và tìm
- Mở: chuỗi lọc = chữ hiện tại (`Combobox` thì xóa). Đóng: xóa (`Autocomplete/AutocompleteRoot.vue:123-143`).
- Mở → highlight item có giá trị bằng chữ; không có thì item đầu (`Autocomplete/AutocompleteRoot.vue:131`, `Listbox/ListboxRoot.vue:441-452`).
- Cùng luật lọc và IME với `Combobox` (`Autocomplete/AutocompleteRoot.vue:157-199`; test `Autocomplete/Autocomplete.test.ts:150-245`).

#### Đóng
- Không có xử lý blur riêng. Focus rời → đóng nhờ lớp dismiss (`Autocomplete/AutocompleteInput.vue:109-130`, `Combobox/ComboboxContentImpl.vue:168-172`).
- Đóng giữ chữ gõ (test `Autocomplete/Autocomplete.test.ts:60-65,144-149`).

#### ARIA và data
- Input như `Combobox` (`Autocomplete/AutocompleteInput.vue:109-122`).
- Look cần đọc: như `Combobox`.

#### Form
- Input ẩn mang chữ (qua `Listbox`) (`Autocomplete/AutocompleteRoot.vue:268-269`; test `Autocomplete/Autocomplete.test.ts:277-312`).

#### RTL
- Như `Combobox`.

#### Cơ chế DOM không port
- Computed ghi được để ép mọi giá trị về chuỗi (`Autocomplete/AutocompleteRoot.vue:108-121`) → giá trị luôn là chữ.

#### Dễ sót
- Chọn item ghi chuỗi của giá trị item, không phải chữ hiển thị.
- Docs ≠ nguồn: "điền chữ của item" (`docs:components/autocomplete.md:104`); nguồn: chuỗi của giá trị item.
- Docs ≠ nguồn: bảng phím nói focus trên item (`docs:components/autocomplete.md:377-399`); nguồn: `aria-activedescendant`, focus ở ô.

---

### Tree
`Tree/` · dùng bởi: không có trong shadcn-vue.

#### Part
- Gốc, Item (một hàng), Virtualizer.

#### Trạng thái
- **Giá trị**: một item, hoặc danh sách khi chọn nhiều (`Tree/TreeRoot.vue:115-120`).
- **Mở rộng**: danh sách khóa. Mặc định `[]` (`Tree/TreeRoot.vue:122-127`).
- Người dùng ghi cả hai (chọn, mở/thu). Caller ghi cả hai. Caller có thể từ chối từng lần chọn và từng lần mở/thu (`Tree/TreeItem.vue:132-149`).
- Cách chọn: bật/tắt (mặc định) hoặc thay (`Tree/TreeRoot.vue:91-95`, `shared/useSelectionBehavior.ts:11-39`).
  - Một + bật/tắt: chọn lại → bỏ.
  - Chế độ một lưu bản sao nông của item; so bằng khóa (`shared/useSelectionBehavior.ts:28-35`, `Tree/TreeRoot.vue:244`).
- Chọn nhiều, tùy chọn: lan xuống (chọn/bỏ mọi con cháu) và lan lên (cha được chọn khi đủ con) (`Tree/TreeRoot.vue:219-270`). Cha có trạng thái nửa chọn (`Tree/TreeItem.vue:70-85`).
- "Có con" = hàm lấy con trả về giá trị khác rỗng. Mảng rỗng vẫn tính là có con (`Tree/TreeRoot.vue:141,150`, `Tree/TreeItem.vue:58`).

#### Bàn phím

| Phím | Kết quả |
|---|---|
| `ArrowDown` / `ArrowUp` | Hàng hiện kế / trước, bỏ hàng vô hiệu. Vòng: tắt (`Tree/TreeRoot.vue:300-307`, `RovingFocus/RovingFocusItem.vue:61-101`). |
| `Home` / `End`, `PageUp` / `PageDown` | Hàng đầu / cuối (`RovingFocus/utils.ts:9-18`). |
| `ArrowRight` (LTR) | Đang thu → mở. Đang mở → focus con đầu. Lá → không gì (`Tree/TreeItem.vue:89-110`). |
| `ArrowLeft` (LTR) | Đang mở → thu. Khác → focus cha (`Tree/TreeItem.vue:112-130`). |
| `Enter` / `Space` | Chọn theo cách chọn (`Tree/TreeItem.vue:197`). |
| `Shift` + `ArrowUp` / `ArrowDown` | Nhiều + thay: chọn dải từ mốc (`Tree/TreeRoot.vue:204-217`, `shared/useSelectionBehavior.ts:41-68`). |
| Ký tự in được | Typeahead: focus hàng khớp, 1000 ms (`Tree/TreeRoot.vue:191-202`). |
| Phím trong `input`/`textarea`/`select`/vùng sửa được trong hàng | Tree bỏ qua (`Tree/TreeRoot.vue:173-189,192,205`). |

#### Focus
- Roving, một điểm Tab (nhóm khác mô tả `RovingFocus`).

#### Pointer
- Click hàng = chọn VÀ mở/thu (`Tree/TreeItem.vue:200-203`).

#### Lọc và tìm
- Chỉ typeahead.

#### Đóng
- Không có popup.

#### ARIA và data
- `role="tree"`; `aria-multiselectable` khi chọn nhiều (`Tree/TreeRoot.vue:309-312`).
- `role="treeitem"`, `aria-selected`, `aria-expanded` chỉ khi có con, `aria-level`, `aria-disabled` (`Tree/TreeItem.vue:186-192`).
- `aria-setsize` / `aria-posinset` theo nhóm anh em (`Tree/TreeRoot.vue:151-156`).
- Look cần đọc: được chọn, đang mở, vô hiệu, cấp (thụt lề), nửa chọn.

#### Form
- Không có.

#### RTL
- Trái/phải đảo (`Tree/TreeItem.vue:198-199`).

#### Ảo hóa
- 28 px mỗi hàng, dư 12 hàng (`Tree/TreeVirtualizer.vue:82-89`). Phím do virtualizer xử lý (`Tree/TreeVirtualizer.vue:121-181`).

#### Cơ chế DOM không port
- Tìm cha/con theo thuộc tính thụt lề trên DOM (`Tree/TreeItem.vue:97-104,121-128`) → `ArrowRight`/`ArrowLeft` tới con/cha.

#### Dễ sót
- Click vừa chọn vừa mở/thu.
- `ArrowRight` trên hàng đang mở đi vào con đầu.
- Mảng con rỗng vẫn là nút có con.
- Docs: bảng phím khớp nguồn (`docs:components/tree.md`).

---

### Ghi chú chung cho nhóm 5
- **Hai mô hình focus.**
  - Roving: Select (item), `Listbox` không Filter, Tree. Focus thật nằm trên item.
  - Focus ở ô chữ: Combobox, Autocomplete, `Listbox` + Filter (shadcn `command`) → `aria-activedescendant`. TagsInput → `aria-current`.
  - S6 phải phơi ra cả hai cho AccessKit: focus thật và active descendant.
- **Highlight trong look**: shadcn Select đọc focus (`shadcn-style:style-vega.css:979-980`); combobox và command đọc trạng thái highlight (`:300-302,385-386`). Với S6 cả hai là một trạng thái "highlight".
- **Thứ tự** = thứ tự hiển thị. Item vô hiệu bị bỏ khi điều hướng và typeahead, vẫn có trong cây AT với `aria-disabled` (`Collection/Collection.ts:39-54`).
- **Bằng nhau** (`Select/utils.ts:17-31`, `Listbox/utils.ts:16-30`): `undefined` → không bằng; chuỗi so `===`; caller cho tên khóa hoặc hàm; còn lại so sâu.
  - Chọn dải dùng so sâu, bỏ qua luật của caller (`shared/arrays.ts:57-66`).
  - TagsInput so `===` (`TagsInput/TagsInputRoot.vue:164`). Tree so bằng khóa (`Tree/TreeRoot.vue:244`).
- **Giá trị `''`** bị cấm cho item Select và Combobox: `''` nghĩa là xóa chọn.
- **Typeahead chung** (`shared/useTypeahead.ts`): tự xóa sau 1000 ms; lặp một chữ → xoay vòng; tiền tố; không phân biệt hoa thường; một ký tự → bỏ item hiện tại; nhiều ký tự → giữ item hiện tại nếu còn khớp. Select dùng cả khi đóng (chọn) và khi mở (focus).
- **Lọc chung** (`shared/useFilter.ts:19-65`): "chứa chuỗi". Collator cố định `'en'`, không theo locale người dùng; `usage: 'search'`; độ nhạy `base` → không phân biệt hoa thường và dấu. Chuẩn hóa NFC. Quét cửa sổ cùng độ dài UTF-16 (`:48-62`). Dùng ở Combobox, Autocomplete, shadcn `command`.
- **IME** (`shared/useComposing.ts`): không lọc, thêm, chọn, điều hướng từ chuỗi đang soạn. Ngoại lệ Android, chữ Latin đang soạn → chạy ngay.
- **Input ẩn cho form** (`VisuallyHidden/VisuallyHiddenInput.vue:15-57`): giá trị nguyên thủy → `name`; danh sách → `name[i]`; object → `name[khóa]`; danh sách object → `name[i][khóa]`; rỗng + bắt buộc → một input rỗng bắt buộc. Select khác: dùng `<select>` gốc, giá trị rỗng thay bằng giá trị thay thế.
- **Popup**: Select kiểu modal (chặn nhấn ngoài, khóa cuộn, ẩn phần còn lại khỏi AT, chặn `Tab`, focus rời không đóng). Combobox mặc định không modal (nhấn ngoài lọt qua, trang cuộn, focus rời thì đóng).
- **Ghi của caller**: Select chỉ đổi "có giá trị" của `Field`. Combobox reset chữ nếu đang đóng. `Listbox` highlight, cuộn, có thể kéo focus.
- **Ảo hóa** (`Listbox`, Tree: 28 px, dư 12). Bảo đảm cần giữ dù S6 ảo hóa cách khác: tổng số và vị trí cho AT; hàng highlight luôn sống; highlight theo option, không theo hàng; không cuộn trang khi mount.
- **Chỗ S6 có thể tốt hơn reka** (ghi vào `verdict/better.tsv`, hoặc ADR nếu là luật):
  - `aria-multiselectable` cho Select chọn nhiều.
  - Ô Filter của `command`: thêm role combobox và quan hệ với danh sách.
  - `End` khi chưa highlight → item cuối.
  - Chọn tất cả nhất quán giữa ảo và không ảo.
  - TagsInput: dán thì trim và bỏ đoạn rỗng; nút xóa tag có tên hành động; `Home` khi chưa chọn → tag đầu.
  - Look `command` trong Chrome không mờ item vô hiệu, không hiện check: quyết định khớp hay sửa.

## 10. Nhóm 6: điều khiển form

Hai chế độ chung cho mọi primitive (trừ Progress): caller sở hữu state, hoặc primitive tự giữ từ một giá trị đầu. Khi caller sở hữu, primitive chỉ đề xuất giá trị mới; state hiển thị chỉ đổi khi caller ghi lại (`Checkbox/CheckboxRoot.vue:157-159`; test `Checkbox/Checkbox.test.ts:528-543`).

### Checkbox
`Checkbox/` · dùng bởi: `checkbox` (Root + Indicator). Group và parent checkbox: không component shadcn-vue nào dùng. Checkbox item trong `dropdown-menu`, `context-menu`, `menubar` là primitive Menu riêng, không phải primitive này.

- **Part:**
  - Root: control vai trò `checkbox`, một tab stop.
  - Indicator: phần trang trí, chỉ có mặt khi checked hoặc indeterminate; có thể ép luôn có mặt để animate; không nhận pointer (`Checkbox/CheckboxIndicator.vue:29-36`).
  - Group: vùng chứa nhiều checkbox cùng ghi một danh sách giá trị; mặc định là một nhóm roving focus (`Checkbox/CheckboxGroupRoot.vue:61-63,96-102`).
  - Parent checkbox: một Root trong Group, điều khiển danh sách "tất cả" do caller cho (`Checkbox/CheckboxRoot.vue:17-22`).
- **Trạng thái:**
  - Checkbox lẻ sở hữu một giá trị: giá trị-đúng, giá trị-sai, hoặc `indeterminate`. Mặc định giá trị-đúng `true`, giá trị-sai `false`. Không có giá trị đầu thì là giá trị-sai (`Checkbox/CheckboxRoot.vue:70-76,103-106`).
  - Checked khi giá trị bằng giá trị-đúng (so cấu trúc) (`Checkbox/CheckboxRoot.vue:136`). Mọi giá trị khác, trừ `indeterminate`, là unchecked.
  - Người dùng ghi (click, Space): `indeterminate` → giá trị-đúng; checked → giá trị-sai; còn lại → giá trị-đúng (`Checkbox/CheckboxRoot.vue:181-187`). Người dùng không tạo ra `indeterminate`; chỉ caller (docs:components/checkbox.md:145-147).
  - Trong Group, checkbox con không có state riêng. Checked = giá trị của nó có trong danh sách (`Checkbox/CheckboxRoot.vue:139-143`). Click thêm giá trị vào cuối hoặc bỏ nó (`Checkbox/CheckboxRoot.vue:172-178`). Danh sách mặc định rỗng (`Checkbox/CheckboxGroupRoot.vue:72-75`).
  - Group có thể giới hạn số mục tối đa; không đặt hoặc `null` = không giới hạn (`Checkbox/CheckboxGroupRoot.vue:19-24`; test `Checkbox/Checkbox.test.ts:319-329`).
  - Parent: state suy ra. Đủ mọi giá trị "tất cả" → checked; không có giá trị nào, hoặc danh sách "tất cả" rỗng → unchecked; còn lại → indeterminate (`Checkbox/utils.ts:18-27`).
- **Bàn phím:**

| Phím | Kết quả |
|---|---|
| Space | Đảo trạng thái. Qua kích hoạt gốc của button; reka không tự bắt Space. |
| Enter | Không làm gì. Bị chặn theo WAI-ARIA (`Checkbox/CheckboxRoot.vue:318-320`). |
| Mũi tên (Group, roving bật) | Chuyển focus, không đổi giá trị. Không vòng (mặc định tắt). Orientation không đặt → cả hai trục (`Checkbox/CheckboxGroupRoot.vue:81-83`, `RovingFocus/RovingFocusGroup.vue:66-70`, `RovingFocus/utils.ts:32-43`). |
| Home/PageUp, End/PageDown (Group) | Focus checkbox đầu / cuối (`RovingFocus/utils.ts:9-18`). |
| Tab (Group, roving bật) | Cả nhóm là một tab stop. Vào nhóm: focus checkbox được focus gần nhất, chưa có thì checkbox không disabled đầu tiên (`RovingFocus/RovingFocusGroup.vue:103-112`). |

- **Pointer:** click như Space. Disabled, hoặc bị chặn bởi giới hạn: bỏ qua (`Checkbox/CheckboxRoot.vue:151-155`).
- **Giá trị:**
  - So giá trị theo cấu trúc, object so sâu (`Checkbox/CheckboxRoot.vue:136`, `shared/isValueEqualOrExist.ts:13-22`; test `Checkbox/Checkbox.test.ts:573-596`).
  - Giới hạn: khi danh sách đã đủ số tối đa, checkbox chưa checked không thêm được (`Checkbox/CheckboxRoot.vue:108-116`). Bỏ một mục thì mở lại (test `Checkbox/Checkbox.test.ts:273-281`).
  - Click parent (`Checkbox/utils.ts:35-57`):
    - Parent chưa checked → thêm các giá trị "tất cả" của checkbox không disabled, theo thứ tự danh sách, dừng khi chạm giới hạn.
    - Parent checked, hoặc việc thêm không đổi gì (chỉ còn mục disabled chưa chọn) → bỏ mọi giá trị không disabled (`Checkbox/utils.ts:52-56`; test `Checkbox/Checkbox.test.ts:618-626`).
    - Giá trị ngoài danh sách "tất cả" và giá trị của checkbox disabled giữ nguyên (test `Checkbox/Checkbox.test.ts:556-563,605-616`).
    - Parent không bị giới hạn chặn (`Checkbox/CheckboxRoot.vue:111-112`; test `Checkbox/Checkbox.test.ts:786-793`).
- **ARIA:**
  - Root: role `checkbox`; `aria-checked` = `true` / `false` / `mixed` (`Checkbox/CheckboxRoot.vue:306,310`); `aria-required` (`:311`).
  - Bị chặn bởi giới hạn: `aria-disabled="true"`, nhưng vẫn focus được và vẫn trong điều hướng mũi tên (`Checkbox/CheckboxRoot.vue:118-123,314-317`; test `Checkbox/Checkbox.test.ts:236-258`). Disabled thật: không focus được, roving bỏ qua.
  - Tên: từ label trỏ tới control, nếu không có tên tường minh (`Checkbox/CheckboxRoot.vue:206-214`).
  - Parent: `aria-controls` = id các checkbox nó điều khiển; tự sinh id còn thiếu (`Checkbox/CheckboxRoot.vue:256-263,265-292`; test `Checkbox/Checkbox.test.ts:651-663`).
  - Group không có role `group` (`Checkbox/CheckboxGroupRoot.vue:96-112`). Caller tự đặt nhóm và nhãn.
  - Look cần đọc: checked / unchecked / indeterminate; disabled thật; "bị chặn bởi giới hạn" là trạng thái riêng, khác disabled thật (reka khuyên vẽ riêng nó và báo người dùng về giới hạn, `Checkbox/CheckboxGroupRoot.vue:19-23`).
- **Form:**
  - Checkbox lẻ có tên: checked → gửi `tên=giá-trị`, mặc định `on`; unchecked → không gửi gì; disabled → không gửi (test `Checkbox/Checkbox.test.ts:110-124,366-391`).
  - Indeterminate được gửi như checked (`Checkbox/CheckboxRoot.vue:334`).
  - Required: checkbox lẻ phải checked thì form mới submit (`Checkbox/CheckboxRoot.vue:331-340`; test `Checkbox/Checkbox.test.ts:98-108`).
  - Group có tên: một mục cho mỗi giá trị, tên `tên[i]`; giá trị object → `tên[i][khóa]` (`VisuallyHidden/VisuallyHiddenInput.vue:29-37`; test `Checkbox/Checkbox.test.ts:420-432`). Danh sách rỗng → không gửi gì.
  - Group required và rỗng: chặn submit, kể cả khi group không có tên. Group disabled thì không chặn (`Checkbox/CheckboxGroupRoot.vue:77-79`; `Checkbox/CheckboxGroupNativeValidation.browser.test.ts:34-65`).
  - Group gửi cả danh sách: checkbox con disabled đang checked vẫn được gửi (`Checkbox/CheckboxGroupRoot.vue:105-111`).
  - Parent không gửi giá trị riêng (`Checkbox/CheckboxRoot.vue:332`; test `Checkbox/Checkbox.test.ts:795-805`).
  - Field: checkbox lẻ (ngoài Group) là control của Field. Nó lấy id, tên, nhãn, mô tả, required, disabled, invalid từ Field. Với Field, "đã điền" = checked đúng `true`; indeterminate không tính (`Checkbox/CheckboxRoot.vue:98-134,195-198,216-229,241-253`).
  - Trong Group, cả nhóm chung một Field: checkbox không lấy id và nhãn của Field, nhưng vẫn lấy disabled, required, invalid, mô tả; blur báo "touched" (`Checkbox/CheckboxRoot.vue:119,124-127,216-229,234-239`).
- **RTL:** chỉ Group: ArrowLeft/ArrowRight đổi chiều (`RovingFocus/utils.ts:20-28`).
- **Cơ chế DOM không port:**
  - Input checkbox ẩn đặt cạnh control, chỉ khi nằm trong form và có tên → submit, required gốc, sự kiện change cho form (`Checkbox/CheckboxRoot.vue:200,331-340`, `shared/useFormControl.ts:5-8`).
  - Ghi `checked` vào input ẩn qua setter gốc rồi phát `input`/`change` → form thấy state mới ngay trong sự kiện change (`VisuallyHidden/VisuallyHiddenInputBubble.vue:30-51`).
  - `querySelector([for=id]).innerText` → tên truy cập lấy từ label (`Checkbox/CheckboxRoot.vue:206-214`).
  - `type=button`, `disabled` gốc → click không submit form; disabled không nhận focus (`Checkbox/CheckboxRoot.vue:309,316`).
  - `Presence` chờ animation thoát → Indicator biến mất sau khi animation xong.
  - `useId`, id `reka-checkbox-late-N` → id ổn định cho `aria-controls` (`Checkbox/CheckboxRoot.vue:53-54,96,278-290`).
  - Chuyển scope id cho CSS scoped (`Checkbox/CheckboxRoot.vue:201-204`) → không có bảo đảm người dùng.
- **Dễ sót:**
  - Indeterminate + click → checked, không phải unchecked (`Checkbox/CheckboxRoot.vue:182-184`; test `Checkbox/Checkbox.test.ts:339-342`).
  - Enter không đảo Checkbox (Switch thì có).
  - Bị chặn bởi giới hạn ≠ disabled: vẫn focus được, screen reader vẫn tới được.
  - Parent: check-all không đổi gì thì chuyển sang uncheck.
  - Group gửi `tên[i]`, không lặp `tên` như HTML thường.
  - Docs lệch: "input render khi trong form" (docs:components/checkbox.md:61). Source còn cần tên và không nằm trong Group (`Checkbox/CheckboxRoot.vue:332`).

### Switch
`Switch/` · dùng bởi: `switch` (Root + Thumb).

- **Part:**
  - Root: control vai trò `switch`, một tab stop.
  - Thumb: phần trang trí, phản ánh cùng state (`Switch/SwitchThumb.vue:19-27`).
- **Trạng thái:**
  - Một giá trị: giá trị-bật (mặc định `true`) hoặc giá trị-tắt (mặc định `false`). Không có giá trị đầu thì là giá trị-tắt (`Switch/SwitchRoot.vue:52-58,72-75`).
  - Bật khi giá trị `===` giá trị-bật (`Switch/SwitchRoot.vue:77`). Mọi giá trị khác, kể cả `null`, là tắt.
  - Người dùng ghi: bật → giá trị-tắt; tắt → giá trị-bật (`Switch/SwitchRoot.vue:79-84`). Không có indeterminate.
- **Bàn phím:**

| Phím | Kết quả |
|---|---|
| Space | Đảo. Qua kích hoạt gốc của button. |
| Enter | Đảo. Bắt riêng ở keydown, chặn mặc định (`Switch/SwitchRoot.vue:117`; test `Switch/Switch.test.ts:40-50`). |

- **Pointer:** click đảo. Disabled bỏ qua (`Switch/SwitchRoot.vue:80-81`).
- **Giá trị:** so `===`; object chỉ khớp khi cùng tham chiếu. Checkbox thì so cấu trúc.
- **ARIA:** role `switch`, `aria-checked` true/false, `aria-required` (`Switch/SwitchRoot.vue:104,108-109`). Tên từ label trỏ tới id (`Switch/SwitchRoot.vue:91`). Disabled bằng disabled gốc, không có `aria-disabled`.
  - Look cần đọc: bật/tắt; disabled; Thumb đọc cùng hai trạng thái đó.
- **Form:** có tên → bật: `tên=giá-trị`, mặc định `on`; tắt → không gửi; disabled → không gửi; required → phải bật (`Switch/SwitchRoot.vue:125-134`; test `Switch/Switch.test.ts:74-96`). Không tham gia Field.
- **RTL:** không có hành vi theo hướng.
- **Cơ chế DOM không port:**
  - Input checkbox ẩn cạnh control → submit, required, change.
  - Tra label theo `for` → tên truy cập.
  - `type=button`; `value` gắn trên button (`Switch/SwitchRoot.vue:105-106`) → không có ý nghĩa cho người dùng.
- **Dễ sót:**
  - Enter có đảo.
  - So `===` với giá trị-bật.
  - Giá trị `null` hoặc lạ = tắt; click → giá trị-bật.

### RadioGroup
`RadioGroup/` · dùng bởi: `radio-group` (Root + Item + Indicator). Rating của reka xây trên nó (`Rating/RatingRoot.vue:4`).

- **Part:**
  - Root: vai trò `radiogroup`, một nhóm roving focus (`RadioGroup/RadioGroupRoot.vue:91-106`).
  - Item: vai trò `radio` (`RadioGroup/Radio.vue:84`).
  - Indicator: chỉ có mặt khi Item được chọn; có thể ép có mặt (`RadioGroup/RadioGroupIndicator.vue:28-30`).
- **Trạng thái:**
  - Root sở hữu một giá trị: giá trị của Item được chọn, hoặc rỗng. Không có giá trị đầu thì rỗng (`RadioGroup/RadioGroupRoot.vue:68-71`).
  - Người dùng ghi: chọn Item (click, Space, mũi tên). Người dùng không bỏ chọn được: click Item đang chọn vẫn chọn (`RadioGroup/Radio.vue:69`; test `RadioGroup/RadioGroup.test.ts:238-247`).
  - Caller ghi bất kỳ giá trị, kể cả rỗng.
  - Caller có thể phủ quyết từng lần chọn: sự kiện "select" hủy được; hủy thì không chọn (`RadioGroup/Radio.vue:64-67`, `RadioGroup/RadioGroupItem.vue:116`).
  - Cấu hình: disabled cả nhóm; orientation (mặc định không đặt); vòng (mặc định bật); required (`RadioGroup/RadioGroupRoot.vue:51-56`).
- **Bàn phím:**

| Phím | Kết quả |
|---|---|
| Tab | Vào nhóm: focus Item đang chọn, không có thì Item không disabled đầu tiên. Tab không chọn (`RadioGroup/RadioGroupItem.vue:107`, `RovingFocus/RovingFocusGroup.vue:103-112`; `RadioGroup/RadioGroup.browser.test.ts:62-73`). |
| Space | Chọn Item đang focus. |
| Enter | Không làm gì (`RadioGroup/RadioGroupItem.vue:117`). |
| ArrowDown / ArrowRight | Focus Item kế **và chọn nó**. |
| ArrowUp / ArrowLeft | Focus Item trước **và chọn nó** (`RadioGroup/RadioGroupItem.vue:61-94`; test `RadioGroup/RadioGroup.test.ts:57-63`). |
| Home/PageUp, End/PageDown | Focus Item đầu / cuối, **không chọn** (`RovingFocus/utils.ts:9-18`; cờ chọn chỉ bật cho 4 mũi tên, `RadioGroup/RadioGroupItem.vue:62`). |

  - Orientation dọc: chỉ Up/Down. Ngang: chỉ Left/Right. Không đặt: cả hai (`RovingFocus/utils.ts:32-43`).
  - Vòng: cuối → đầu. Item disabled bị bỏ qua (test `RadioGroup/RadioGroup.test.ts:46-49`).
  - Mũi tên kèm Ctrl, Alt, Meta hoặc Shift: không điều hướng (`RovingFocus/RovingFocusItem.vue:80-81`).
- **Pointer:** click chọn. Disabled bỏ qua (`RadioGroup/Radio.vue:61-62`; test `RadioGroup/RadioGroup.test.ts:164-167`).
- **Giá trị:**
  - So cấu trúc (`RadioGroup/RadioGroupItem.vue:57`).
  - Disabled của Item = disabled nhóm hoặc disabled Item; required tương tự (`RadioGroup/RadioGroupItem.vue:55-56`).
  - Gõ mũi tên nhanh: Item cuối cùng giữ focus được chọn. Focus rời đi trước lúc chọn thì không chọn (test `RadioGroup/RadioGroup.test.ts:67-114`; `RadioGroup/RadioGroup.browser.test.ts:38-60`).
- **ARIA:**
  - Root: `radiogroup`; `aria-orientation` chỉ khi đặt; `aria-required` (`RadioGroup/RadioGroupRoot.vue:99-105`).
  - Item: `radio`, `aria-checked` (`RadioGroup/Radio.vue:84,87`). Tên từ label trỏ tới id; không dùng giá trị làm tên (`RadioGroup/Radio.vue:58`; test `RadioGroup/RadioGroup.test.ts:174-185`).
  - Look cần đọc: mỗi Item chọn / không chọn, disabled; nhóm disabled.
- **Form:**
  - Nhóm có tên → một mục `tên=giá-trị-đang-chọn` (`RadioGroup/RadioGroupRoot.vue:109-115`). Giá trị object → `tên[khóa]` (`VisuallyHidden/VisuallyHiddenInput.vue:41-43`).
  - Chưa chọn → gửi `tên=""`, không vắng mặt. Suy từ source: trường ẩn là ô text với giá trị rỗng (`VisuallyHidden/VisuallyHiddenInput.vue:22-26`); không có test.
  - Required và chưa chọn → chặn submit. Nhóm disabled → không gửi.
  - Item disabled đang được chọn vẫn được gửi, vì giá trị thuộc nhóm (suy từ `RadioGroup/RadioGroupRoot.vue:109-115`).
- **RTL:** ArrowLeft/ArrowRight đổi chiều (`RovingFocus/utils.ts:20-28`; nhóm nhận hướng ở `RadioGroup/RadioGroupRoot.vue:94`).
- **Cơ chế DOM không port:**
  - Cờ "đang nhấn mũi tên" bắt ở capture phase, reset sau `nextTick`; chọn bằng `click()` trong `setTimeout(0)` sau khi focus tới → mũi tên chọn Item nhận focus cuối cùng (`RadioGroup/RadioGroupItem.vue:61-94`).
  - `stopPropagation` click của button khi trong form → form chỉ thấy một sự kiện change, từ input ẩn (`RadioGroup/Radio.vue:70-75`).
  - Input ẩn của nhóm; input radio ẩn riêng của Item chỉ khi Item có tên riêng (`RadioGroup/Radio.vue:102-112`).
  - Đọc `activeElement` từ root node → chạy được trong shadow root (`RadioGroup/RadioGroupItem.vue:89-91`).
- **Dễ sót:**
  - ArrowUp/ArrowLeft cũng chọn.
  - Home/End chỉ focus, không chọn.
  - Người dùng không bỏ chọn được.
  - Caller phủ quyết được việc chọn.
  - Vòng mặc định bật (CheckboxGroup mặc định tắt).
  - Docs lệch: ArrowUp/ArrowLeft "chỉ chuyển focus" (docs:components/radio-group.md:132-139). Test và source: chuyển focus và chọn. Docs bỏ sót Home/End.

### Slider
`Slider/` · dùng bởi: `slider` (Root, Track, Range, một Thumb cho mỗi giá trị: `shadcn:slider/Slider.vue`). ColorSlider của reka xây trên nó (`ColorSlider/ColorSliderRoot.vue:57`).

- **Part:**
  - Root: vùng nhận pointer và phím; không có role (`Slider/SliderImpl.vue:36-82`).
  - Track: nền của dải; chứa Range.
  - Range: đoạn tô giữa các giá trị.
  - Thumb: một Thumb cho mỗi giá trị; vai trò `slider`; mỗi Thumb một tab stop (`Slider/SliderThumbImpl.vue:71-84`). Chỉ số Thumb = thứ tự trong cây (`Slider/SliderThumb.vue:20`, `Slider/SliderThumbImpl.vue:47-53`).
- **Trạng thái:**
  - Root sở hữu danh sách số, mỗi Thumb một số. Mỗi lần người dùng ghi, danh sách được sắp tăng dần (`Slider/utils.ts:19-23`). Giá trị đầu mặc định `[0]` (`Slider/SliderRoot.vue:88`). Không phải danh sách (vd `null`) = rỗng (`Slider/SliderRoot.vue:114`).
  - Thumb đang hoạt động = Thumb được focus gần nhất (`Slider/SliderThumbImpl.vue:97-99`).
  - Người dùng ghi bằng pointer hoặc phím. Lần ghi bị bỏ nếu không đổi gì hoặc vi phạm khoảng cách tối thiểu (`Slider/SliderRoot.vue:143-152`).
  - Caller được báo hai loại: (1) mỗi lần danh sách đổi, liên tục khi kéo; (2) "commit" khi xong tương tác: mỗi phím có đổi giá trị, hoặc một lần lúc thả pointer nếu Thumb đã đổi giá trị (`Slider/SliderRoot.vue:40-51,128-134,146-147`).
  - Cấu hình (mặc định): min 0, max 100, step 1; khoảng cách tối thiểu giữa Thumb 0 bước; ngang; không đảo; Thumb nằm gọn trong Track (`Slider/SliderRoot.vue:81-92`).
- **Bàn phím** (Thumb đang focus; mọi phím dưới đây chặn cuộn trang, `Slider/SliderImpl.vue:39-55`):

| Phím | Kết quả (ngang, LTR, không đảo) |
|---|---|
| Tab | Qua từng Thumb. Disabled: Thumb không focus được (`Slider/SliderThumbImpl.vue:75`). |
| ArrowRight / ArrowUp | +1 step |
| ArrowLeft / ArrowDown | −1 step |
| PageUp / PageDown | ±10 step |
| Shift + bất kỳ mũi tên | ×10, cả Left/Right (`Slider/SliderRoot.vue:194-196`) |
| Home | Thumb **đầu tiên** (chỉ số 0) về min, bất kể Thumb nào có focus; focus chuyển sang nó (`Slider/SliderRoot.vue:150,190`). |
| End | Thumb **cuối cùng** về max (`Slider/SliderRoot.vue:191`). |

  - Phím "lùi" theo hướng trượt (`Slider/utils.ts:127-132`, `Slider/SliderHorizontal.vue:85-89`, `Slider/SliderVertical.vue:81-85`):
    - Từ trái (ngang LTR; hoặc RTL + đảo): lùi = ArrowLeft, ArrowDown, PageDown.
    - Từ phải (ngang RTL; hoặc LTR + đảo): lùi = ArrowRight, ArrowDown, PageDown. ArrowLeft tăng (test `Slider/Slider.test.ts:89-105`).
    - Từ dưới (dọc không đảo): lùi = ArrowDown, ArrowLeft, PageDown.
    - Từ trên (dọc + đảo): lùi = ArrowUp, ArrowLeft, PageUp. ArrowDown và PageDown tăng (test `Slider/Slider.test.ts:190-224`).
    - Home/End không bao giờ đảo (test `Slider/Slider.test.ts:143-150,226-234`).
- **Pointer:**
  - Nhấn trên Thumb: chỉ focus Thumb đó; giá trị chưa đổi. Khi Thumb nằm gọn, kéo giữ điểm đã nắm nằm dưới con trỏ. Ở chế độ tràn, tâm Thumb theo con trỏ (`Slider/SliderImpl.vue:64-66`, `Slider/SliderHorizontal.vue:31-36,46`; `Slider/Slider.browser.test.ts:68-80`).
  - Nhấn trên Track: Thumb gần nhất theo khoảng cách giá trị nhảy tới; bằng nhau → chỉ số nhỏ hơn. Tâm Thumb đặt dưới con trỏ, rồi kéo tiếp. Thumb đó nhận focus (`Slider/SliderRoot.vue:119-122,150`, `Slider/utils.ts:54-60`, `Slider/SliderHorizontal.vue:44-45`; test `Slider/Slider.test.ts:600-623`).
  - Kéo qua Thumb khác: danh sách sắp lại; focus và Thumb đang hoạt động đi theo giá trị đang kéo, sang Thumb kia (`Slider/SliderRoot.vue:141-151`).
  - Kéo tiếp khi con trỏ ra ngoài slider, tới khi thả. Mỗi pointer kết thúc đúng một lần, kể cả mất capture trước khi thả, hoặc bị hủy. Nhiều pointer kết thúc riêng (`Slider/SliderImpl.vue:27-32,56-81`; test `Slider/Slider.test.ts:341-401`).
  - Ánh xạ vị trí ↔ giá trị khi Thumb nằm gọn: chiều dài dùng được = chiều dài − kích thước Thumb; ở min mép đầu Thumb chạm đầu Track, ở max mép cuối chạm cuối (`Slider/SliderHorizontal.vue:31,39-41`, `Slider/utils.ts:66-71`). Chế độ tràn: tâm Thumb đúng tỉ lệ; Thumb tràn nửa kích thước ra hai đầu.
  - Không lọc nút chuột: nút phụ cũng kéo (`Slider/SliderImpl.vue:56-70`). Quirk reka.
- **Giá trị:**
  - Snap: `round((v − min) / step) · step + min`, làm tròn tới số chữ số thập phân lớn nhất của min và step, rồi kẹp vào [min, max] (`Slider/SliderRoot.vue:137-139`, `Slider/utils.ts:115-119`). Lưới tính từ min: min 0.5, step 1 → 1.5 (test `Slider/Slider.test.ts:24-39`).
  - Max ngoài lưới (min 0, max 10, step 3): End cho 9; mũi tên từ 9 kẹp lên 10. Suy từ `Slider/SliderRoot.vue:137-139`; không có test.
  - Khoảng cách tối thiểu: mọi cặp kề nhau cách ít nhất (số bước × step). Vi phạm thì bỏ cả lần ghi, không kẹp sát (`Slider/SliderRoot.vue:143`, `Slider/utils.ts:96-103`). Một bước nhảy lớn qua hẳn Thumb kia vẫn được nhận nếu khoảng cách mới đủ.
  - Khoảng cách so bằng số thực thô (`Slider/utils.ts:96-101`). Với step thập phân có thể từ chối sai (0.3 − 0.2 < 0.1). S6 nên so theo đơn vị bước.
  - Không đổi → không báo (`Slider/SliderRoot.vue:145`).
- **ARIA:**
  - Thumb: `slider`, `aria-valuenow`, `aria-valuemin`, `aria-valuemax`, `aria-orientation` (`Slider/SliderThumbImpl.vue:74-82`).
  - Tên mặc định khi không có nhãn: 2 Thumb → "Minimum" / "Maximum"; hơn 2 → "Value i of n"; 1 → không có (`Slider/utils.ts:35-44`).
  - Không có `aria-valuetext`.
  - Disabled: Thumb mất tab stop; chỉ Root (không role) có `aria-disabled`, Thumb thì không (`Slider/SliderThumbImpl.vue:75`, `Slider/SliderRoot.vue:182`).
  - Look cần đọc: vị trí mỗi Thumb (tỉ lệ trên trục, cộng bù khi nằm gọn); đoạn Range (1 Thumb: từ đầu tới giá trị; nhiều Thumb: từ giá trị nhỏ nhất tới lớn nhất, `Slider/SliderRange.vue:19-24`); orientation; hướng trượt (để căn Thumb, `Slider/SliderHorizontal.vue:68-71`); disabled.
- **Form:** có tên → một mục số cho mỗi giá trị, tên `tên[i]` (test `Slider/Slider.test.ts:496-499`: `slider[0]=50`). Disabled → không gửi.
- **RTL:** ngang RTL trượt từ phải, ArrowLeft tăng. RTL + đảo trượt từ trái (`Slider/SliderHorizontal.vue:22`). Dọc bỏ qua hướng (`Slider/SliderVertical.vue:19`).
- **Cơ chế DOM không port:**
  - Pointer capture, `lostpointercapture` → kéo tiếp ngoài vùng; kết thúc đúng một lần mỗi pointer.
  - `getBoundingClientRect`, `clientWidth`, ResizeObserver → ánh xạ theo kích thước thật của Root và Thumb (`Slider/SliderHorizontal.vue:27-31`, `Slider/SliderThumbImpl.vue:33-34`).
  - Biến CSS transform, `calc(% + px)` → vị trí Thumb (`Slider/SliderThumbImpl.vue:85-96`).
  - Chặn focus mặc định lúc pointerdown, rồi `focus()` thủ công → Thumb nhận focus khi giá trị đổi, cả trên cảm ứng (`Slider/SliderImpl.vue:60-66`, `Slider/SliderRoot.vue:150`).
  - Ẩn Thumb chưa có giá trị trước khi mount → tránh nhảy vị trí khi SSR.
  - Input `type=number` ẩn → form.
  - `compareDocumentPosition` → chỉ số Thumb theo thứ tự cây.
- **Dễ sót:**
  - Home/End tác động Thumb đầu / cuối, không phải Thumb đang focus.
  - Shift + mũi tên nào cũng ×10.
  - Commit: phím → mỗi phím có đổi giá trị; pointer → một lần khi thả, chỉ khi Thumb đã đổi.
  - Thumb disabled không mang `aria-disabled`. S6 nên đặt disabled trên chính node slider.
  - Docs lệch:
    - "Đảo chiều thì đảo cả Home/End" (docs:components/slider.md:321). Source không đảo. Tên test `Slider/Slider.test.ts:226,231` cũng nói ngược với assert.
    - "Một input cho mỗi Thumb" (docs:components/slider.md:59). Thực tế: một mục cho mỗi giá trị, tên `tên[i]`.
    - Bảng phím chỉ nêu Shift + Up/Down (docs:components/slider.md:299-306). Source: Shift + mọi mũi tên.

### Toggle
`Toggle/` · dùng bởi: `toggle`. ToggleGroup Item bọc Toggle bên trong (`ToggleGroup/ToggleGroupItem.vue:41-48`).

- **Part:** một nút bật/tắt; một tab stop.
- **Trạng thái:** một boolean "đang nhấn", mặc định `false` (`Toggle/Toggle.vue:41-45,68-71`). Người dùng ghi: click đảo (`Toggle/Toggle.vue:73-75`). Caller ghi bất kỳ boolean.
- **Bàn phím:**

| Phím | Kết quả |
|---|---|
| Space | Đảo. Qua kích hoạt gốc của button; reka không tự bắt phím (docs:components/toggle.md:71-78). |
| Enter | Đảo, như trên. |

- **Pointer:** click đảo. Disabled chặn được chỉ nhờ disabled gốc của button; hàm đảo không tự kiểm tra (`Toggle/Toggle.vue:73-75,93`).
- **Giá trị:** boolean.
- **ARIA:** `aria-pressed` true/false (`Toggle/Toggle.vue:90`); disabled gốc.
  - Look cần đọc: nhấn / không nhấn; disabled.
- **Form:** có tên và không nằm trong ToggleGroup → nhấn: `tên=true`; không nhấn: không gửi; disabled: không gửi, kể cả khi đang nhấn (`Toggle/Toggle.vue:105-114`; test `Toggle/Toggle.test.ts:97-124`).
- **RTL:** không có.
- **Cơ chế DOM không port:**
  - Input checkbox ẩn → submit; form thấy state mới trong sự kiện change (test `Toggle/Toggle.test.ts:106-115`).
  - `type=button` → không submit form khi click.
- **Dễ sót:**
  - Giá trị gửi là `true`, không phải `on`.
  - Disabled phải chặn mọi đường kích hoạt. Reka dựa vào button gốc, nên khi Toggle không phải button thì vẫn đảo được lúc disabled (`Toggle/Toggle.vue:73-75`). S6 không chép.

### ToggleGroup
`ToggleGroup/` · dùng bởi: `toggle-group` (Root + Item). Toolbar của reka xây trên nó (`Toolbar/ToolbarToggleGroup.vue:14`).

- **Part:**
  - Root: vai trò `group`; một nhóm roving focus khi bật (`ToggleGroup/ToggleGroupRoot.vue:80-104`).
  - Item: một Toggle mang giá trị (`ToggleGroup/ToggleGroupItem.vue:35-51`).
- **Trạng thái:**
  - Kiểu: một (single) hoặc nhiều (multiple). Caller chỉ định; không chỉ định thì suy từ giá trị: danh sách → nhiều, còn lại → một (`shared/useSingleOrMultipleValue.ts:18-33`).
  - Một: một giá trị hoặc rỗng. Click Item khác → chọn nó. Click Item đang chọn → rỗng (`shared/useSingleOrMultipleValue.ts:53-56`; test `ToggleGroup/ToggleGroup.test.ts:34-46`). Muốn luôn có giá trị: caller từ chối lần ghi rỗng (docs:components/toggle-group.md:92-115).
  - Nhiều: danh sách. Click thêm giá trị vào cuối hoặc bỏ nó (`shared/useSingleOrMultipleValue.ts:57-67`).
  - Mặc định: một → rỗng; nhiều → danh sách rỗng (`shared/useSingleOrMultipleValue.ts:35-40`).
  - Cấu hình: roving focus (mặc định bật); vòng (mặc định bật); orientation (mặc định không đặt); disabled cả nhóm (`ToggleGroup/ToggleGroupRoot.vue:47-51`).
- **Bàn phím** (roving bật):

| Phím | Kết quả |
|---|---|
| Tab | Một tab stop. Vào nhóm: focus Item đang nhấn (đầu tiên theo thứ tự), không có thì Item không disabled đầu tiên (`ToggleGroup/ToggleGroupItem.vue:39`). |
| Space / Enter | Đảo Item đang focus (kích hoạt gốc). |
| Mũi tên | Chỉ chuyển focus, không nhấn. Vòng. Theo orientation. Bỏ qua Item disabled (`RovingFocus/RovingFocusItem.vue:61-101`; test `ToggleGroup/ToggleGroup.test.ts:48-79`). |
| Home/PageUp, End/PageDown | Focus Item đầu / cuối (`RovingFocus/utils.ts:9-18`). |

  - Roving tắt: mỗi Item là một tab stop; mũi tên không làm gì (`ToggleGroup/ToggleGroupRoot.vue:81-87`, `ToggleGroup/ToggleGroupItem.vue:36-40`).
- **Pointer:** click = đảo Item đó trong nhóm. Disabled của Item = disabled nhóm hoặc disabled Item (`ToggleGroup/ToggleGroupItem.vue:26`).
- **Giá trị:**
  - So cấu trúc (`ToggleGroup/ToggleGroupItem.vue:27`, `shared/useSingleOrMultipleValue.ts:55`).
  - Kiểu nhiều nhưng giá trị hiện tại không phải danh sách: giá trị falsy (`0`, `''`) bị bỏ khi đổi sang danh sách (`shared/useSingleOrMultipleValue.ts:58`). Quirk.
  - Không chỉ định kiểu và không có giá trị nào: nhóm hành xử như kiểu một, nhưng giá trị đầu là danh sách rỗng (`shared/useSingleOrMultipleValue.ts:25,39`). Với form: lúc đầu không gửi gì; chọn rồi bỏ chọn thì gửi `tên=""`. Quirk; suy từ source.
- **ARIA:**
  - Root `group`, không có `aria-orientation` (`ToggleGroup/ToggleGroupRoot.vue:90`).
  - Item dùng `aria-pressed`, kể cả ở kiểu một; không có ngữ nghĩa radio (`Toggle/Toggle.vue:90`).
  - Look cần đọc: mỗi Item nhấn / không nhấn, disabled; orientation của nhóm. Variant, size, spacing là của look shadcn (`shadcn:toggle-group/ToggleGroup.vue`).
- **Form:**
  - Có tên, kiểu một: `tên=giá-trị` (test `ToggleGroup/ToggleGroup.test.ts:193-197`). Rỗng → `tên=""` (suy từ `VisuallyHidden/VisuallyHiddenInput.vue:22-26`).
  - Kiểu nhiều: `tên[i]` cho mỗi giá trị. Rỗng → không gửi. Rỗng + required → chặn submit (`VisuallyHidden/VisuallyHiddenInput.vue:15-37`).
  - Nhóm disabled → không gửi (test `ToggleGroup/ToggleGroup.test.ts:199-203`). Item không tự gửi (`Toggle/Toggle.vue:106`).
- **RTL:** ArrowLeft/ArrowRight đổi chiều (roving, `RovingFocus/utils.ts:20-28`).
- **Cơ chế DOM không port:** input ẩn của nhóm → submit; tabindex của roving → một tab stop.
- **Dễ sót:**
  - Kiểu một cho phép rỗng (RadioGroup thì không).
  - Mũi tên không nhấn (RadioGroup thì chọn).
  - Kiểu suy từ hình dạng giá trị khi caller không chỉ định.
  - Docs lệch: bảng phím thiếu PageUp/PageDown (docs:components/toggle-group.md:160-168).

### NumberField
`NumberField/` · dùng bởi: `number-field` (Root, Input, Increment, Decrement). `NumberFieldContent` của shadcn chỉ là vùng bố cục (`shadcn:number-field/NumberFieldContent.vue`).

- **Part:**
  - Root: vai trò `group` (`NumberField/NumberFieldRoot.vue:292`).
  - Input: ô văn bản vai trò `spinbutton`; tab stop duy nhất (`NumberField/NumberFieldInput.vue:113-115`).
  - Increment / Decrement: nút ±; không nằm trong tab order; nhấn-giữ lặp (`NumberField/NumberFieldIncrement.vue:30-46`).
- **Trạng thái:**
  - Root sở hữu một số hoặc rỗng; mặc định rỗng (`NumberField/NumberFieldRoot.vue:85-98`).
  - Text hiển thị = số định dạng theo locale và tùy chọn format; rỗng khi số rỗng (`NumberField/NumberFieldRoot.vue:207`). Trong lúc gõ, text là bản nháp; số chưa đổi.
  - Người dùng ghi số bằng: commit text (blur, Enter); bước (mũi tên, Page, nút, wheel); Home/End.
  - Caller được báo mỗi lần số đổi; xóa được báo là "không có giá trị" (`NumberField/NumberFieldRoot.vue:44-49`).
  - Cấu hình (mặc định) (`NumberField/NumberFieldRoot.vue:9-42,85-91`):
    - step 1; snap theo lưới: bật; min, max: không có.
    - Cho phép giữ giá trị gõ ngoài phạm vi / ngoài lưới: tắt.
    - Giữ giá trị khi text không parse được: tắt.
    - Focus Input khi bước: bật.
    - Locale: theo ngữ cảnh, mặc định `en` (`shared/useLocale.ts:10-15`).
    - Wheel: bật, không đảo. Disabled, readonly: tắt.
    - Giá trị khởi đầu khi ô rỗng: min, hoặc 0.
- **Bàn phím** (Input):

| Phím | Kết quả |
|---|---|
| ArrowUp / ArrowDown | ±1 bước |
| PageUp / PageDown | ±10 bước |
| Home | Về min nếu có min. Luôn nuốt phím: con trỏ text không về đầu. |
| End | Về max nếu có max. Luôn nuốt phím. |
| Enter | Commit text. Không chặn mặc định (`NumberField/NumberFieldInput.vue:55-57`). |
| Phím khác | Gõ text, có lọc (xem Giá trị). |

  - Nguồn: `NumberField/NumberFieldInput.vue:23-59`. Đang soạn IME: mọi phím trên bị bỏ qua (`NumberField/NumberFieldInput.vue:27-28`; test `NumberField/NumberField.test.ts:1022-1035`). Không có hành vi ArrowLeft/Right riêng (di chuyển con trỏ text).
- **Pointer:**
  - Nút (chỉ nút chính, `button 0`) (`NumberField/utils.ts:62`):
    - Chuột: +1 bước ngay khi nhấn. Giữ 400 ms thì lặp mỗi 60 ms. Thả thì dừng (`NumberField/utils.ts:8-9,73-78`; test `NumberField/NumberField.test.ts:178-187`).
    - Chạm / bút: không bước khi nhấn. Thả trước 400 ms → +1 bước (tap). Giữ yên 400 ms → +1, rồi mỗi 60 ms (`NumberField/utils.ts:80,95-104`; test `NumberField/NumberField.test.ts:189-243`).
    - Chạm dịch quá 10 px (khoảng cách Euclid) → hủy, trước hay trong khi lặp. Dịch ít hơn vẫn lặp. Bị hủy bởi hệ thống → hủy (`NumberField/utils.ts:10,83-93,106-109`; test `NumberField/NumberField.test.ts:212-287`).
    - Lặp dừng khi nút thành disabled, vd chạm giới hạn (`NumberField/utils.ts:40-48`, `NumberField/NumberFieldIncrement.vue:20`).
    - Nút không nằm trong tab order. Mặc định mỗi lần bước, focus chuyển vào Input; caller tắt được (`NumberField/NumberFieldIncrement.vue:34`, `NumberField/NumberFieldRoot.vue:152-154`; test `NumberField/NumberField.test.ts:942-955`).
    - Giữ lâu không mở menu ngữ cảnh, không chọn chữ (`NumberField/NumberFieldIncrement.vue:37-39,43`).
  - Wheel trên Input (`NumberField/NumberFieldInput.vue:69-89`; test `NumberField/NumberField.test.ts:341-386`):
    - Chỉ khi Input đang focus.
    - Bỏ qua nếu |ΔY| ≤ |ΔX| (cuộn chủ yếu ngang).
    - Mỗi sự kiện một bước, bất kể độ lớn ΔY.
    - ΔY > 0 (cuộn xuống) → **tăng**; ΔY < 0 → giảm. Có thể đảo hoặc tắt.
    - Khi xử lý thì chặn cuộn trang.
- **Giá trị:**
  - Bước đọc text đang có trong ô, kể cả bản nháp chưa commit; không đọc số đã lưu (`NumberField/NumberFieldRoot.vue:157`; test `NumberField/NumberField.test.ts:698-718`).
  - Ô rỗng hoặc không parse được, rồi bước (cả tăng lẫn giảm) → đặt giá trị khởi đầu. Caller cho giá trị khởi đầu → kẹp [min, max], không snap. Không cho → min (hoặc 0), có kẹp và snap (`NumberField/NumberFieldRoot.vue:158-166`; test `NumberField/NumberField.test.ts:122-175,586-592`).
  - Đang trên lưới: ±step × bội số, cộng trừ thập phân chính xác (`NumberField/NumberFieldRoot.vue:130-131`, `NumberField/utils.ts:134-157`).
  - Ngoài lưới (snap bật): trước hết về đường lưới theo hướng bước (18.98 ↑ → 19; 18.11 ↓ → 18). PageUp/Down cộng thêm (bội số − 1) bước (`NumberField/NumberFieldRoot.vue:132-142`; test `NumberField/NumberField.test.ts:532-545`).
  - Kết quả bước luôn được kẹp [min, max] và snap (`NumberField/NumberFieldRoot.vue:148,223-233`).
  - Snap: lưới tính từ min, không có min thì từ 0. Làm tròn về đường lưới gần nhất. Vượt max → đường lưới cuối cùng ≤ max (`shared/clamp.ts:68-90`). Vì vậy max ngoài lưới không với tới khi snap bật (min 2, max 21, step 3 → 20), với tới khi snap tắt (→ 21) (test `NumberField/NumberField.test.ts:505-530`).
  - Số chữ thập phân của lưới = lớn nhất của step và min (`shared/clamp.ts:26-42`; test `NumberField/NumberField.test.ts:481-491`).
  - Mọi số người dùng ghi được chuẩn hóa theo độ chính xác hiển thị: format rồi parse lại (`NumberField/NumberFieldRoot.vue:165,231,251`). Số chữ thập phân tối đa của format làm tròn chính giá trị.
  - Giá trị đang ngoài phạm vi (khi cho phép): bước ra xa phạm vi → không làm gì; bước về phía phạm vi → kẹp tới biên (`NumberField/NumberFieldRoot.vue:170-173`; test `NumberField/NumberField.test.ts:638-659`).
  - Nút ± disabled khi bước không thể cho giá trị lớn hơn / nhỏ hơn. Ô rỗng → hai nút luôn bật (`NumberField/NumberFieldRoot.vue:106-117`; test `NumberField/NumberField.test.ts:557-584`).
  - Commit text, khi blur hoặc Enter (`NumberField/NumberFieldRoot.vue:238-263`):
    - Rỗng hoặc chỉ khoảng trắng → xóa giá trị, luôn luôn.
    - Không còn chữ số nào (`.`, `-`, `$`, `%`), hoặc không parse được (`1.2.3`, `abc`) → xóa. Với tùy chọn giữ: giữ số cũ, khôi phục text định dạng, không báo caller (test `NumberField/NumberField.test.ts:739-869`).
    - Parse được → kẹp và snap; hoặc giữ nguyên nếu cho phép, chỉ chuẩn hóa độ chính xác. Text thay bằng bản định dạng: `7` → `EUR 7.00` (test `NumberField/NumberField.test.ts:720-737`).
    - "Chữ số" gồm mọi hệ chữ số Unicode và số Hán `〇一二三四五六七八九` (`NumberField/NumberFieldRoot.vue:235-236`).
  - Lọc khi gõ: lần chèn bị từ chối nếu text kết quả không phải số dở dang hợp lệ với locale và format. Có xét dấu theo min/max: min ≥ 0 thì không gõ được `-`, trừ khi cho phép giá trị ngoài phạm vi (`NumberField/NumberFieldRoot.vue:209-216`, `NumberField/NumberFieldInput.vue:130-144`; test `NumberField/NumberField.test.ts:661-695`). Xóa và undo/redo không bao giờ bị chặn (`NumberField/NumberFieldInput.vue:134`). Không lọc khi đang soạn IME (test `NumberField/NumberField.test.ts:996-1007`).
  - Locale khi parse: hiểu dấu nhóm và dấu thập phân của locale (`de-DE` `1.234` → 1234); hệ chữ số của locale (`ar-EG` `١٢٣`, hanidec `一二三`); và chữ số hệ khác ngay cả trong `en-US` (`٢٥` → 0.25 với percent) (test `NumberField/NumberField.test.ts:815-836,927-939`).
  - Format: currency, unit, percent, signDisplay, số chữ thập phân, accounting (test `NumberField/NumberField.test.ts:388-478`).
  - Percent: số là phân số (0.05 hiển thị `5%`). Step mặc định vẫn là 1 (= 100%); caller phải tự đặt 0.01 (docs:components/number-field.md:151).
  - Readonly: chặn bước và nút, vẫn focus được (test `NumberField/NumberField.test.ts:318-338`). Nhưng Home/End vẫn ghi min/max khi readonly: lỗi reka, `NumberField/NumberFieldRoot.vue:184-189` không xét readonly hay disabled. S6 không chép.
- **ARIA:**
  - Root `group`.
  - Input: `spinbutton`; `aria-roledescription="Number field"`; `aria-valuenow` = số thô, vắng khi rỗng; `aria-valuemin`, `aria-valuemax`; không có `aria-valuetext` (`NumberField/NumberFieldInput.vue:113-127`).
  - Nút: nhãn cố định "Increase" / "Decrease"; disabled gốc (`NumberField/NumberFieldIncrement.vue:35,40`, `NumberField/NumberFieldDecrement.vue:35,40`).
  - Look cần đọc: disabled, readonly của field; mỗi nút: disabled (kể cả do chạm giới hạn), đang nhấn.
- **Form:** có tên → `tên=<số thô>` (vd `5`, `0.05`), không phải text định dạng. Rỗng → `tên=""`. Disabled → không gửi (`NumberField/NumberFieldRoot.vue:304-312`; test `NumberField/NumberField.test.ts:957-993`). Required + rỗng → chặn, trừ khi readonly: trường ẩn cũng readonly, và HTML không kiểm tra trường readonly (`NumberField/NumberFieldRoot.vue:310`; suy từ source).
- **RTL:** không có hành vi theo hướng.
- **Cơ chế DOM không port:**
  - `@internationalized/number` (NumberFormatter, NumberParser, dựa trên `Intl`) → hiển thị, parse, kiểm tra số dở dang theo locale (`NumberField/utils.ts:126-132`). Thư viện không có trong `references/`; S6 cần nguồn CLDR riêng và nên đọc thư viện này trước khi thiết kế.
  - `beforeinput` + ghép với vùng chọn → lọc ký tự khi gõ.
  - `inputmode` decimal / numeric theo format có phần thập phân hay không → bàn phím ảo phù hợp (`NumberField/NumberFieldRoot.vue:195-203`).
  - `autocomplete`, `autocorrect`, `spellcheck` tắt → không gợi ý, không tự sửa (`NumberField/NumberFieldInput.vue:121-123`).
  - `requestAnimationFrame` sau `change`, đồng bộ lại text sau commit → bản nháp cũ không hiện lại (`NumberField/NumberFieldInput.vue:61-67,100-104`).
  - Listener pointer trên `window`, `setTimeout` → nhấn-giữ với các thời lượng trên.
  - Chặn `contextmenu`, `user-select: none` → giữ lâu không mở menu, không chọn chữ.
  - Input text ẩn → form.
- **Dễ sót:**
  - Bước đọc text nháp, không đọc số đã lưu.
  - Ô rỗng + bước (kể cả giảm) → giá trị khởi đầu, không phải ±step.
  - Ngoài lưới: căn lưới theo hướng trước.
  - Max ngoài lưới không với tới khi snap.
  - Giá trị bị làm tròn theo format.
  - Wheel xuống = tăng; chỉ khi focus.
  - Chuột bước ngay khi nhấn; chạm bước khi thả.
  - Docs lệch: bảng phím thiếu Enter và wheel. Docs nói Home/End "nếu có min/max" (docs:components/number-field.md:247-255); source nuốt phím cả khi không có (`NumberField/NumberFieldInput.vue:47-54`).

### PinInput
`PinInput/` · dùng bởi: `pin-input` (Root; Input đổi tên thành "Slot"). shadcn bật OTP mặc định (`shadcn:pin-input/PinInput.vue`). `input-otp` của shadcn-vue dùng vue-input-otp, không dùng primitive này.

- **Part:**
  - Root: vùng chứa; không role, không nhãn; mang hướng (`PinInput/PinInputRoot.vue:152-158`).
  - Input (ô): mỗi ô một ký tự, gắn với một vị trí cố định trong giá trị. Mỗi ô là một tab stop (`PinInput/PinInputInput.vue:8-13,283-311`).
  - Trường ẩn: mang giá trị cho form; nhận focus (vd từ label) rồi chuyển sang ô đầu (`PinInput/PinInputRoot.vue:161-172`).
- **Trạng thái:**
  - Root sở hữu danh sách theo vị trí ô: ký tự (kiểu text) hoặc số (kiểu number). Mặc định rỗng (`PinInput/PinInputRoot.vue:9-14,95-101`).
  - Ô đang không hiển thị vẫn giữ giá trị của vị trí nó. "Đủ" và giá trị form chỉ xét ô đang hiển thị (`PinInput/PinInputRoot.vue:104-122`; `PinInput/PinInput.slots.browser.test.ts:22-35`).
  - "Đủ": mọi ô hiển thị có giá trị. Text: khác rỗng. Number: có số, kể cả 0 (`PinInput/PinInputRoot.vue:123-127`).
  - Caller được báo "đủ", kèm các giá trị hiển thị, mỗi lần giá trị đổi trong khi đang đủ (`PinInput/PinInputRoot.vue:129-132`).
  - Cấu hình: kiểu text / number (mặc định text); ẩn ký tự (mask); OTP; placeholder (mặc định rỗng); disabled cả nhóm hoặc từng ô (`PinInput/PinInputRoot.vue:29-48,78-81`, `PinInput/PinInputInput.vue:27`).
- **Bàn phím:**

| Phím | Kết quả |
|---|---|
| Ký tự | Ô nhận ký tự mới, thay ký tự cũ dù con trỏ ở đâu; focus sang ô kế. Ở ô cuối, gõ tiếp thay ký tự ô cuối (`PinInput/PinInputInput.vue:89-92`; test `PinInput/PinInput.test.ts:37-76`). |
| Backspace | Ô có ký tự → xóa, ở lại. Ô rỗng → về ô trước và xóa nó (`PinInput/PinInputInput.vue:128-144`; test `PinInput/PinInput.test.ts:130-153,319-338`). |
| Delete | Xóa ô hiện tại, ở lại; không dồn ký tự (`PinInput/PinInputInput.vue:146-151`; test `PinInput/PinInput.test.ts:155-167`). |
| ArrowLeft / ArrowRight | Ô trước / kế. Không vòng. Bỏ qua ô disabled (`PinInput/PinInputInput.vue:110-126`, `shared/useArrowNavigation.ts:127-133,168-205`). |
| Home / End | Ô đầu / ô cuối. |
| ArrowUp / ArrowDown | Không điều hướng; để hành vi mặc định (`shared/useArrowNavigation.ts:105-113`). |
| Tab / Shift+Tab | Qua từng ô (test `PinInput/PinInput.test.ts:537-558`). |
| Dán | Xem "Nhập nhiều ký tự". |

  - Kiểu number: ký tự không phải chữ số ASCII bị bỏ; ô giữ nguyên (`PinInput/PinInputInput.vue:31-32,84-87`; test `PinInput/PinInput.test.ts:251-269`).
  - IME: trong khi soạn, không chuyển ô, không điều hướng. Chỉ xử lý ký tự đã chốt. Chốt nhiều ký tự → rải ra các ô (`PinInput/PinInputInput.vue:43-72,75,112`; test `PinInput/PinInput.test.ts:388-475`).
- **Nhập nhiều ký tự** (dán, autofill, IME chốt nhiều ký tự) (`PinInput/PinInputInput.vue:203-239`):
  - Kiểu number: bỏ mọi ký tự không phải chữ số trước (`a1b2c3` → `123`) (test `PinInput/PinInput.test.ts:271-304`).
  - Độ dài ≥ số ô → điền từ ô đầu. Ngắn hơn → điền từ ô hiện tại. Phần dư bị cắt (`PinInput/PinInputInput.vue:218-221`; test `PinInput/PinInput.test.ts:78-97`).
  - Focus tới ô sau ô cuối được điền, hoặc ở ô cuối.
  - Trình quản lý mật khẩu điền từng ô bằng sự kiện input cũng được nhận (test `PinInput/PinInput.test.ts:370-385`).
- **Pointer:** click vào ô = focus ô đó. OTP: xem dưới.
- **OTP** (`PinInput/PinInputInput.vue:153-197`):
  - Không focus được ô nằm sau ô rỗng đầu tiên. Click, hoặc focus từ ngoài vào, chuyển về ô rỗng đầu tiên (test `PinInput/PinInput.test.ts:495-519`).
  - Mũi tên và End không vượt quá ô rỗng đầu tiên (test `PinInput/PinInput.test.ts:521-535`).
  - Tab / Shift+Tab giữa các ô không bị chuyển hướng: không bẫy bàn phím (`PinInput/PinInputInput.vue:183-186`; test `PinInput/PinInput.test.ts:537-558`).
  - Gợi ý hệ điều hành tự điền mã một lần (`PinInput/PinInputInput.vue:289`).
- **Giá trị:**
  - Text: mỗi ô một chuỗi; mọi ký tự, kể cả khoảng trắng, đều nhận. Chuỗi rỗng ở cuối danh sách bị cắt (`PinInput/PinInputInput.vue:241-250,265-269`).
  - Number: số 0–9. Ô bị xóa thành "không có", giữ chỗ trong danh sách, không bị cắt (`PinInput/PinInputInput.vue:255-264`; test `PinInput/PinInput.test.ts:340-354`).
  - Nhiều ký tự được tách theo đơn vị UTF-16, không theo grapheme (`PinInput/PinInputInput.vue:216-239`). Giới hạn reka; S6 tách theo grapheme.
- **ARIA:**
  - Mỗi ô có nhãn "pin input i of n", tiếng Anh, theo thứ tự hiển thị (`PinInput/PinInputInput.vue:298`; `PinInput/PinInput.slots.browser.test.ts:71-81`).
  - Root không có role hay nhãn.
  - Placeholder ẩn ở ô đang focus và rỗng (`PinInput/PinInputInput.vue:95-108`; test `PinInput/PinInput.test.ts:29-35`). Focus đặt con trỏ ở cuối ô (`PinInput/PinInputInput.vue:195`).
  - Look cần đọc: mỗi ô: ký tự (hoặc ký tự che khi mask), placeholder có hiện không, focus, disabled; nhóm: đủ, disabled.
- **Form:**
  - Luôn có trường, để required và focus từ label hoạt động. Không có tên thì không gửi gì, vì tên rỗng (`PinInput/PinInputRoot.vue:161-172`).
  - Giá trị = nối ký tự các ô hiển thị (`1234`). Lỗ hổng bị nuốt: `[1, ∅, 3]` → `13` (`PinInput/PinInputRoot.vue:167`).
  - Required + rỗng → chặn. Disabled → không gửi.
- **RTL:** ArrowLeft = ô kế, ArrowRight = ô trước. Home/End theo thứ tự cây (`shared/useArrowNavigation.ts:128`, `PinInput/PinInputInput.vue:124`).
- **Cơ chế DOM không port:**
  - Mỗi ô là một `<input>` gốc; `setSelectionRange(1,1)` → con trỏ cuối ô.
  - Đổi thuộc tính placeholder lúc focus/blur → placeholder ẩn ở ô đang focus và rỗng.
  - `type=password` → mask.
  - `inputmode=numeric`, `pattern=[0-9]*` → bàn phím ảo số (`PinInput/PinInputInput.vue:291-292`).
  - `autocomplete=one-time-code` → hệ điều hành tự điền mã.
  - Sự kiện `paste` + clipboard → dán.
  - `compareDocumentPosition` + WeakMap → thứ tự ô theo cây; ô ↔ vị trí giá trị.
  - Trường ẩn `aria-hidden` nhận focus rồi chuyển sang ô đầu → label và validation gốc trỏ đúng ô.
- **Dễ sót:**
  - Backspace ở ô rỗng xóa ô trước.
  - Dán: từ ô đầu nếu đủ dài, ngược lại từ ô hiện tại.
  - "Đủ" được báo lại mỗi lần đổi khi đang đủ.
  - Form nối chuỗi; lỗ hổng bị nuốt.
  - Dán ghi cả vào ô disabled (`PinInput/PinInputInput.vue:216-239`). S6 không chép.
  - Docs lệch: End "tới ô cuối" (docs:components/pin-input.md:142-144); OTP thì tới ô rỗng đầu tiên. "Input render khi trong form" (docs:components/pin-input.md:53); thực tế luôn render.

### Progress
`Progress/` · dùng bởi: `progress` (Root + Indicator).

- **Part:**
  - Root: vai trò `progressbar`.
  - Indicator: phần trang trí; không có ARIA (`Progress/ProgressIndicator.vue:18-26`).
- **Trạng thái:**
  - Caller ghi giá trị (số trong [0, max]) hoặc "không xác định" (rỗng), và max (mặc định 100) (`Progress/ProgressRoot.vue:13-30,80-84`). Người dùng không ghi.
  - Giá trị sai (NaN, < 0, > max, không phải số) → coi là không xác định; ghi ngược về caller; cảnh báo cho dev (`Progress/ProgressRoot.vue:45-60,105-108`; test `Progress/Progress.test.ts:37-74`).
  - Max sai (≤ 0, NaN) → 100; báo caller (`Progress/ProgressRoot.vue:62-72,100-103`; test `Progress/Progress.test.ts:76-87`).
  - Trạng thái suy ra: không xác định (rỗng); hoàn tất (giá trị == max); đang chạy (còn lại) (`Progress/ProgressRoot.vue:110-116`).
- **Bàn phím:** không có.
- **Pointer:** không có.
- **Giá trị:** min cố định 0. Giá trị không bị làm tròn.
- **ARIA:**
  - `progressbar`; `aria-valuemin=0`; `aria-valuemax`; `aria-valuenow` (vắng khi không xác định); `aria-valuetext` chỉ khi caller cho hàm tạo chữ (`Progress/ProgressRoot.vue:126-134`).
  - Tên mặc định = phần trăm làm tròn (`80%`); vắng khi không xác định (`Progress/ProgressRoot.vue:82-83,133`). Đây là tên, không phải valuetext.
  - Look cần đọc: không xác định / đang chạy / hoàn tất; tỉ lệ giá trị / max.
- **Form:** không có.
- **RTL:** không có trong primitive.
- **Cơ chế DOM không port:** `console.error` → cảnh báo dev khi giá trị hoặc max sai.
- **Dễ sót:**
  - Không xác định khi rỗng hoặc sai; hoàn tất chỉ khi đúng bằng max.
  - Tên mặc định là phần trăm. S6 nên cân nhắc tên từ label và phần trăm làm valuetext.
  - shadcn-vue dịch Indicator theo giá trị thô như phần trăm, tức giả định max 100 (`shadcn:progress/Progress.vue`).
  - Docs lệch: "Indicator giúp AT đọc tiến độ" (docs:components/progress.md:78). Indicator không có ARIA; Root mang hết.

### Stepper
`Stepper/` · dùng bởi: `stepper` (đủ 7 part).

- **Part:**
  - Root: vai trò `group`, tên cố định "progress"; chứa vùng thông báo live (`Stepper/StepperRoot.vue:205-242`).
  - Item: một bước; đánh dấu bước hiện tại (`Stepper/StepperItem.vue:98-111`).
  - Trigger: nút chọn bước (`Stepper/StepperTrigger.vue:118-137`).
  - Indicator: số của bước; nội dung mặc định "Step n" (`Stepper/StepperIndicator.vue:29-31`).
  - Title, Description: tên và mô tả của Trigger.
  - Separator: trang trí, ẩn khỏi AT; theo trạng thái bước chứa nó (`Stepper/StepperSeparator.vue:21-26`, `shared/component/BaseSeparator.vue:41-45`).
- **Trạng thái:**
  - Root sở hữu bước hiện tại: một số, mặc định 1 (`Stepper/StepperRoot.vue:52-57,93-96`).
  - Người dùng ghi qua Trigger. Caller ghi trực tiếp, hoặc qua: đi tới bước, bước kế, bước trước (`Stepper/StepperRoot.vue:121-153`). Caller đọc được: tổng số bước, đang ở đầu / cuối, có bước kế / trước, bước kế / trước có disabled không (`Stepper/StepperRoot.vue:60-85,155-171`).
  - Cấu hình: tuần tự (linear, mặc định bật); orientation (mặc định ngang); thông báo (mặc định bật, chữ tùy biến được) (`Stepper/StepperRoot.vue:52-57`).
  - Item: cờ hoàn tất (mặc định tắt), disabled (`Stepper/StepperItem.vue:29-43`).
  - Trạng thái mỗi Item, theo thứ tự ưu tiên: hoàn tất (cờ) > đang (== hiện tại) > hoàn tất (hiện tại > bước) > chưa (`Stepper/StepperItem.vue:61-69`).
  - Với tới được: disabled → không. Tuần tự → bước ≤ hiện tại, hoặc = hiện tại + 1. Không tuần tự → mọi bước (`Stepper/StepperItem.vue:73-80`). Không với tới được = disabled và rời tab order.
  - Đi tới bước (caller): từ chối < 1 hoặc > tổng. Tuần tự: từ chối > hiện tại + 1 (`Stepper/StepperRoot.vue:121-137`). Kiểm tra disabled ở đây không bao giờ đúng: đọc nhầm chỉ số, so chuỗi rỗng như falsy (`Stepper/StepperRoot.vue:128`). Lỗi reka. Cờ "bước kế / trước disabled" thì đúng (`Stepper/StepperRoot.vue:155-171`).
  - Các helper đầu / cuối / kế / trước giả định bước đánh số 1..n (`Stepper/StepperRoot.vue:100-101,139-153`), trong khi Trigger nhận số bước bất kỳ.
- **Bàn phím** (Trigger):

| Phím | Kết quả |
|---|---|
| Tab | Mỗi Trigger với tới được là một tab stop; không roving (`Stepper/StepperTrigger.vue:128`). |
| Enter / Space (không Ctrl, không Shift) | Chọn bước, ngay khi nhấn xuống (`Stepper/StepperTrigger.vue:89-90`). |
| ArrowLeft/Right (ngang), ArrowUp/Down (dọc) | Focus Trigger trước / kế. Không vòng. Bỏ qua Trigger disabled hoặc không với tới. Không chọn (`Stepper/StepperTrigger.vue:92-100`; test `Stepper/Stepper.test.ts:60-131`). |
| Mũi tên khác trục | Không làm gì; vẫn bị chặn mặc định (`Stepper/StepperTrigger.vue:84`). |
| Home / End | Không xử lý. |

- **Pointer:** nhấn chuột trái chọn bước ngay ở lúc nhấn (mousedown), nếu với tới được. Ctrl + nhấn: không chọn, không lấy focus (`Stepper/StepperTrigger.vue:61-81,132`; test `Stepper/Stepper.test.ts:133-140`).
- **Giá trị:** số bước tùy ý. Vị trí trong thông báo tính theo thứ tự cây, không theo số bước (`Stepper/StepperRoot.vue:106-117`; test `Stepper/StepperAnnouncement.test.ts:29-54`).
- **ARIA:**
  - Root `group` + `aria-label="progress"` (`Stepper/StepperRoot.vue:207-208`).
  - Trigger của bước hiện tại: `aria-current="step"`, kể cả khi bước đã hoàn tất. Item: `aria-current="true"` (`Stepper/StepperTrigger.vue:129`, `Stepper/StepperItem.vue:103`; test `Stepper/Stepper.test.ts:274-330`).
  - Tên Trigger = Title + Description, chỉ những part đang hiển thị. Caller tự đặt tên thì Description thành mô tả (`Stepper/StepperTrigger.vue:37-56`; test `Stepper/Stepper.test.ts:204-312`).
  - Vùng live lịch sự, atomic, role `status`: "Step i of n", i là vị trí 1-based trong các Trigger đang hiển thị. Rỗng khi bước hiện tại không khớp Trigger nào. Caller dịch được hoặc tắt (`Stepper/StepperRoot.vue:104-117,228-242`; test `Stepper/StepperAnnouncement.test.ts:56-121`).
  - Look cần đọc: mỗi bước hoàn tất / đang / chưa; disabled (gồm "không với tới được"); orientation; tuần tự hay không; Separator theo trạng thái bước của nó.
- **Form:** không có.
- **RTL:** ngang: ArrowLeft = Trigger kế (`shared/useArrowNavigation.ts:128`; `Stepper/StepperAnnouncement.browser.test.ts:8-24`).
- **Cơ chế DOM không port:**
  - MutationObserver → thông báo đúng vị trí khi bước bị sắp lại bên trong component con (`Stepper/StepperRoot.vue:200-201`).
  - Vùng live ẩn bằng style (dịch ra ngoài, opacity 0) → chỉ AT nghe thấy (`Stepper/StepperRoot.vue:228-242`).
  - Title là `h4`, Description là `p` (`Stepper/StepperTitle.vue:13`, `Stepper/StepperDescription.vue:13`) → chỉ là thẻ HTML.
  - Id SSR cho Title và Description (`Stepper/StepperTrigger.vue:24-31`).
- **Dễ sót:**
  - Tuần tự chỉ mở tới hiện tại + 1.
  - Chọn ở lúc nhấn chuột, không đợi click.
  - Không roving: mỗi Trigger một tab stop.
  - Thông báo theo vị trí, không theo số bước.
  - Chuỗi tiếng Anh cố định: "progress", "Step n", "Step i of n".
  - Docs lệch: "Tab focus bước đầu tiên" (docs:components/stepper.md:244-246). Source: Tab đi qua từng Trigger với tới được.

---

### input_otp của shadcn-rs — bảo đảm PinInput mà ô OTP cũng cần
Chỉ xét từ source PinInput của reka. vue-input-otp (thư viện shadcn-vue dùng) không được đọc ở đây.

- Thứ tự điền: không focus, click hay mũi tên vượt ô rỗng đầu tiên; Tab không bị bẫy (`PinInput/PinInputInput.vue:153-197`).
- Một ký tự mỗi ô; ký tự mới thay ký tự cũ; tự sang ô kế (`PinInput/PinInputInput.vue:89-92`).
- Nhập nhiều ký tự một lần (dán, autofill hệ điều hành hoặc trình quản lý mật khẩu, IME chốt nhiều ký tự) → rải ra các ô; đủ dài thì từ ô đầu, phần dư bị cắt (`PinInput/PinInputInput.vue:203-239`; test `PinInput/PinInput.test.ts:370-385`).
- Chế độ số: chỉ chữ số; dán thì lọc bỏ ký tự khác trước khi rải (`PinInput/PinInputInput.vue:31-32,84-87,210-212`).
- Backspace ở ô rỗng về ô trước và xóa nó; Delete xóa tại chỗ (`PinInput/PinInputInput.vue:128-151`).
- Phát hiện "đủ" và báo caller; 0 tính là đã điền (`PinInput/PinInputRoot.vue:123-132`).
- Gợi ý hệ điều hành tự điền mã một lần và bàn phím số (`PinInput/PinInputInput.vue:289-292`).
- An toàn IME: không chuyển ô, không điều hướng khi đang soạn (`PinInput/PinInputInput.vue:43-75,112`).
- Form: một giá trị chuỗi nối; required; disabled không gửi (`PinInput/PinInputRoot.vue:161-172`).
- Focus từ label đi vào ô đầu (`PinInput/PinInputRoot.vue:171`).
- Nhãn truy cập mỗi ô theo vị trí "i of n"; S6 bản địa hóa (`PinInput/PinInputInput.vue:298`).
- Placeholder ẩn ở ô đang focus; con trỏ ở cuối ô (`PinInput/PinInputInput.vue:95-108,195`).
- Mask tùy chọn (`PinInput/PinInputInput.vue:290`). Mũi tên theo hướng đọc (`PinInput/PinInputInput.vue:124`).
- Look cần đọc (theo wrapper shadcn-vue, `shadcn:input-otp/InputOTPSlot.vue`): ký tự mỗi ô, ô đang active, có caret giả hay không.

### Ghi chú chung cho nhóm 6
- **So sánh giá trị:** Checkbox, RadioGroup, ToggleGroup so cấu trúc (`shared/isValueEqualOrExist.ts:13-22`). Switch so `===` (`Switch/SwitchRoot.vue:77`). S6 nên chọn một quy tắc.
- **Disabled:** disabled thật = không focus được, không submit, roving bỏ qua. `aria-disabled` chỉ dùng cho checkbox bị chặn bởi giới hạn (vẫn focus được) và Root Slider. Lỗ hổng reka không chép: Toggle không phải button vẫn đảo khi disabled; Thumb Slider thiếu disabled cho AT; Home/End của NumberField khi readonly.
- **Form:**
  - Docs nói trường ẩn "render khi trong form". Source còn cần tên (Checkbox, Switch, RadioGroup, Slider, Toggle, ToggleGroup, NumberField). PinInput luôn render. CheckboxGroup render cả khi không có tên nhưng required và rỗng.
  - Mã hóa: vô hướng → `tên=giá-trị`; danh sách → `tên[i]`; object → `tên[khóa]` hoặc `tên[i][khóa]`; danh sách rỗng + required → một trường rỗng để chặn submit (`VisuallyHidden/VisuallyHiddenInput.vue:15-46`).
  - Checkbox, Switch, Toggle chưa bật → không gửi. RadioGroup và ToggleGroup kiểu một, chưa chọn → `tên=""`. Disabled → không gửi.
- **Tab và mũi tên:**

| Primitive | Tab stop | Mũi tên | Vòng | Vào nhóm tới |
|---|---|---|---|---|
| CheckboxGroup | 1 (roving) | chỉ focus | tắt | item focus gần nhất / đầu tiên |
| RadioGroup | 1 (roving) | focus **và chọn** | bật | item đang chọn |
| ToggleGroup | 1 (roving, tắt được) | chỉ focus | bật | item đang nhấn |
| Slider | mỗi Thumb | đổi giá trị | — | — |
| PinInput | mỗi ô | đổi ô, theo hướng đọc | tắt | — |
| Stepper | mỗi Trigger với tới được | chỉ focus, một trục | tắt | — |

- **Enter:** Checkbox, Radio: không làm gì. Switch, Toggle, ToggleGroup Item: đảo. Stepper: chọn bước. NumberField: commit text.
- **Chuỗi tiếng Anh cố định:** "Minimum", "Maximum", "Value i of n" (Slider); "Increase", "Decrease", "Number field" (NumberField); "pin input i of n" (PinInput); "progress", "Step n", "Step i of n" (Stepper; chỉ thông báo live tùy biến được); "{n}%" (tên Progress). S6 cần bản địa hóa.
- **Hai thuật toán snap khác nhau:** Slider làm tròn về lưới gần nhất rồi kẹp, nên max ngoài lưới vẫn tới được bằng mũi tên (`Slider/SliderRoot.vue:137-139`). NumberField hạ về đường lưới cuối ≤ max (`shared/clamp.ts:79-80`). Cả hai neo lưới ở min và lấy số chữ thập phân của cả min lẫn step.
- **IME:** NumberField và PinInput bỏ qua phím và nhập liệu khi đang soạn (`NumberField/NumberFieldInput.vue:27-28`, `PinInput/PinInputInput.vue:75,112`).
- **Field:** trong nhóm này chỉ Checkbox tham gia Field (`Checkbox/CheckboxRoot.vue:98-101`).
- **Dùng lại trong reka:** Slider → ColorSlider; RadioGroup → Rating; ToggleGroup → Toolbar. Bảo đảm ở đây là nền của các primitive đó.
- **Quan sát look shadcn-vue:** Slider vẽ một Thumb cho mỗi giá trị (`shadcn:slider/Slider.vue`); Progress dịch theo giá trị thô, giả định max 100 (`shadcn:progress/Progress.vue`); PinInput bật OTP mặc định (`shadcn:pin-input/PinInput.vue`).

## 11. Nhóm 7: ngày và giờ

Mọi state ở đây có hai chế độ: caller sở hữu, hoặc primitive tự giữ từ một giá trị đầu.

Thuật ngữ:
- **Ngày neo**: ngày quyết định tháng nào đang hiện và ô nào giữ tab stop; field dùng nó làm nền khi segment rỗng (reka gọi là `placeholder`). Luôn có, kể cả khi chưa chọn gì.
- **Ngày bị chặn**: trước min, sau max, bị caller chặn, hoặc cả lịch disabled (`Calendar/useCalendar.ts:188-196`). Không focus, không chọn, mũi tên nhảy qua.
- **Ngày không khả dụng**: caller đánh dấu, ví dụ đã có người đặt (`Calendar/useCalendar.ts:198-202`). Focus được, không chọn được.
- **Trang**: các tháng đang hiện.
- **Segment**: một phần sửa được của field: ngày, tháng, năm, giờ, phút, giây, AM/PM. Múi giờ là segment chỉ hiện.

Không primitive nào trong nhóm có vùng thông báo (`aria-live`, `role="status"`): grep trong `Calendar/`, `RangeCalendar/`, `DateField/`, `TimeField/`, `DatePicker/`, `DateRangePicker/`, `DateRangeField/`, `shared/date/`, `date/` không ra gì. AT chỉ biết thay đổi qua focus, nhãn và trạng thái ARIA.

### Calendar
`Calendar/` · dùng bởi: `calendar` (`shadcn:calendar/`); ví dụ chọn ngày trong popover (`shadcn-ex:calendar/DatePickerSimple.vue:35-57`, `shadcn-ex:calendar/DatePickerWithDropdowns.vue:36-72`).

- **Part:**
  - Root: vùng lịch có nhãn, chứa một heading ẩn cấp 2 lặp lại nhãn (`Calendar/CalendarRoot.vue:349,364-373`).
  - Heading: chữ của trang; không role, không live (`Calendar/CalendarHeading.vue:23-31`).
  - Prev, Next: nút lật trang (`Calendar/CalendarPrev.vue`, `Calendar/CalendarNext.vue`).
  - Grid (mỗi tháng một lưới), GridHead (hàng tên thứ, ẩn với AT), GridBody, GridRow, HeadCell, Cell, CellTrigger (ô ngày tương tác) (`Calendar/CalendarGrid.vue:22-25`, `Calendar/CalendarGridHead.vue:16`, `Calendar/CalendarCell.vue:23-26`).
  - shadcn-vue thêm select tháng và năm. Chúng là caller ghi vào ngày neo, không phải part (`shadcn:calendar/Calendar.vue:55,77,99-118`). Năm trong select: từ min (không có thì 100 năm trước ngày neo) tới max (không có thì 10 năm sau) (`shadcn:calendar/Calendar.vue:29-37`).
- **Trạng thái:**
  - Giá trị: rỗng, một ngày, hoặc (chế độ nhiều ngày) một danh sách theo thứ tự bấm (`Calendar/CalendarRoot.vue:264-295`).
    - Người dùng ghi bằng click, Enter, Space trên ô (`Calendar/CalendarCellTrigger.vue:103-107,190-192`).
    - Một ngày: chưa có → chọn. Bấm lại đúng ngày đang chọn → bỏ chọn, trừ khi caller cấm bỏ chọn. Ngày khác → thay (`Calendar/CalendarRoot.vue:265-276`; test `Calendar/Calendar.test.ts:178-208`).
    - Nhiều ngày: ngày mới vào cuối danh sách; ngày đã có → bị bỏ; bỏ ngày cuối cùng → giá trị rỗng, không phải danh sách rỗng (`Calendar/CalendarRoot.vue:277-294`; test `Calendar/Calendar.test.ts:1379-1392`).
    - Cấm bỏ chọn chặn mọi lần bỏ, kể cả khi đang chọn nhiều ngày (`Calendar/CalendarRoot.vue:271,285`; test `Calendar/Calendar.test.ts:1394-1410`).
    - "Đã chọn" so theo ngày, bỏ giờ (`Calendar/useCalendar.ts:39-48`).
    - Không ghi khi lịch readonly, khi ngày bị chặn hoặc không khả dụng (`Calendar/CalendarCellTrigger.vue:94-101`).
    - Caller ghi: giữ nguyên, không kẹp vào min/max, không lọc ngày bị chặn; chỉ bật invalid (`Calendar/useCalendar.ts:50-72`).
  - Ngày neo:
    - Giá trị đầu: ngày cuối của giá trị, hoặc ngày neo caller cho, hoặc hôm nay theo hệ lịch mặc định của locale (`shared/date/comparators.ts:30-55`).
    - Người dùng ghi bằng phím điều hướng và Prev/Next (`Calendar/CalendarCellTrigger.vue:121-189`; `Calendar/useCalendar.ts:212-302`).
    - Caller ghi: trang đổi sang tháng chứa ngày neo, chỉ khi tháng đó chưa hiện (`Calendar/useCalendar.ts:304-314`).
    - Giá trị đổi sang ngày khác → ngày neo theo (ngày cuối danh sách). Giá trị mới cùng ngày (đối tượng mới, giờ khác) → giữ trang (`Calendar/CalendarRoot.vue:254-262`; test `Calendar/Calendar.test.ts:91-114`).
  - Tuỳ chọn caller, mặc định (`Calendar/CalendarRoot.vue:124-140,188`): 1 tháng; lật từng tháng (không theo cả trang); không cố định 6 tuần; chọn một ngày; cho bỏ chọn; không disabled, không readonly; không tự focus khi mount; ngày ngoài tháng không bị chặn; tên thứ dạng hẹp; ngày đầu tuần theo locale. Thêm: min, max, hàm chặn ngày, hàm không khả dụng, hàm lật trang riêng, nhãn lịch, locale, hướng.
- **Bàn phím:** trên ô ngày (`Calendar/CalendarCellTrigger.vue:109-226`).

| Phím | Kết quả |
|---|---|
| ArrowLeft / ArrowRight | −1 / +1 ngày; RTL đảo (`:120-127`). |
| ArrowUp / ArrowDown | −7 / +7 ngày (`:128-133`). |
| Home / End | Ngày đầu / cuối của tuần chứa ô, kể cả khi đó là ngày tháng kề (`shared/useCalendarKeyboardNavigation.ts:62-71`; test `Calendar/Calendar.test.ts:809-835`: Home từ 1/1/2021 tới 27/12/2020). |
| PageUp / PageDown | Cùng ngày, tháng trước / sau; ngày không có thì ngày cuối tháng (`shared/useCalendarKeyboardNavigation.ts:97-102`; test `Calendar/Calendar.test.ts:899-915`: 31/3/2024 → 29/2/2024). |
| Shift+PageUp / Shift+PageDown | Cùng ngày, năm trước / sau; 29/2 → 28/2 (`:163,179`; test `Calendar/Calendar.test.ts:950-982`). |
| Enter / Space | Chọn hoặc bỏ chọn ô (`:190-192`). |
| Ctrl / Meta / Alt + Enter / Space | Không xử lý; sự kiện đi lên cha, ví dụ để gửi form (`:112-115`; test `Calendar/Calendar.test.ts:548-582`). |
| Escape | Không xử lý; đi lên cha (popover đóng). |
| Tab | Không xử lý; lưới là một tab stop (xem Focus). |

  - Mũi tên (`Calendar/CalendarCellTrigger.vue:195-225`):
    - Ngày đích ngoài min/max → đứng yên, không kẹp (`:198-199`; test `Calendar/Calendar.test.ts:738-779`).
    - Ngày đích không thuộc tháng nào đang hiện → lật trang (nếu nút tương ứng không bị chặn) rồi thử lại (`:201-217`).
    - Ngày đích bị chặn → bước tiếp cùng hướng, cùng cỡ bước (`:220-222`; test `Calendar/Calendar.test.ts:602-637`: ArrowUp từ 10/1 qua 3/1 bị chặn, tới 27/12).
    - Ngày đích không khả dụng → dừng trên nó (suy từ mã: chỉ kiểm bị chặn, `:220`).
  - Home/End/PageUp/PageDown (`shared/useCalendarKeyboardNavigation.ts:62-198`):
    - Đích kẹp vào min/max (`:76-80,119-132`; test `Calendar/Calendar.test.ts:984-1037`).
    - PageUp/PageDown lật trang khi đích ngoài trang và nút tương ứng không bị chặn (`:87-92`).
    - Đích bị chặn hoặc không khả dụng → dò từng ngày (`:181-191`). Home/End: quay về phía ô xuất phát, ở lại trong tuần. PageUp/PageDown: đi tiếp theo hướng. Đích bị kẹp ở min khi đi lùi → dò tới; kẹp ở max khi đi tới → dò lùi (`:141-154`; test `Calendar/Calendar.test.ts:1039-1141,1193-1230`).
    - Dò tối đa 24 bước rồi thôi (`:13,167-168`; test `Calendar/Calendar.test.ts:1143-1191`).
- **Focus:**
  - Lưới có đúng một tab stop (`shared/useCalendarKeyboardNavigation.ts:15-32`; `Calendar/CalendarCellTrigger.vue:82-90,246`):
    1. Ngày neo, nếu nó trong trang, không bị chặn, không khả dụng, và lịch không disabled.
    2. Nếu không, và chưa chọn gì hoặc ngày chọn bị chặn: ngày đầu tiên focus được của trang, bắt đầu từ min nếu min trong tháng (`Calendar/useCalendar.ts:360-375`; test `Calendar/Calendar.test.ts:1541-1565`).
    3. Ngoài hai trường hợp trên: không có tab stop (suy từ mã, `Calendar/CalendarCellTrigger.vue:87-89`). Ví dụ: ngày đang chọn không khả dụng; hoặc caller dời ngày neo tới ngày bị chặn khi đã có ngày chọn.
  - Ô bị chặn và ô ngoài tháng không nhận Tab (test `Calendar/Calendar.test.ts:1481-1539`). Mũi tên không dừng ở ô ngoài tháng: ngày đó được tìm trong lưới của chính tháng nó, hoặc lật trang (`Calendar/CalendarCellTrigger.vue:83,201`).
  - Ngày neo tới tháng chưa hiện (Home/End, PageUp/PageDown, click ô ngoài tháng) → trang đổi theo (`Calendar/useCalendar.ts:304-314`). PageUp/PageDown có test kiểm chữ trang (`Calendar/Calendar.test.ts:899-915`); Home/End và click: suy từ mã.
  - Nhiều tháng: một ngày có thể hiện hai lần (trong tháng của nó, và ô ngoài tháng ở lưới bên); focus luôn vào bản trong tháng (`shared/useCalendarKeyboardNavigation.ts:170-174`; test `Calendar/Calendar.test.ts:1275-1303`).
  - Focus đầu (tuỳ chọn khi mount; khi picker mở): ngày đã chọn → hôm nay → ngày đầu trong tháng không bị chặn (`shared/date/utils.ts:56-71`; `Calendar/CalendarRoot.vue:297-300`; test `Calendar/Calendar.test.ts:1461-1473`). Không theo ngày neo.
  - Prev/Next không kéo focus khỏi nút; ngày neo về ngày 1 của tháng đầu (`Calendar/useCalendar.ts:228,274`).
  - Nút Prev/Next bị chặn là disabled thật, rời thứ tự Tab (`Calendar/CalendarNext.vue:27,46`).
  - Lịch disabled: không ô nào focus được, Prev/Next disabled (test `Calendar/Calendar.test.ts:639-673`). Readonly: focus và điều hướng như thường, không chọn (test `Calendar/Calendar.test.ts:675-697`).
- **Giá trị:**
  - Ô mang kiểu và giờ của ngày neo, nên ngày được chọn mang giờ của ngày neo (`date/calendar.ts:65`; `Calendar/CalendarRoot.vue:267,275`).
  - min/max so bằng `compare` đầy đủ, có giờ (`Calendar/useCalendar.ts:188-196`); phím điều hướng so theo ngày, bỏ giờ (`shared/useCalendarKeyboardNavigation.ts:104-107`).
  - Chặn ngày ngoài tháng (tuỳ chọn): ô ngoài tháng thành bị chặn (`Calendar/CalendarCellTrigger.vue:80`).
  - Invalid: có ngày đang chọn bị chặn hoặc không khả dụng (`Calendar/useCalendar.ts:50-72`).
  - Không tham gia form: không có input ẩn (grep `input` trong `Calendar/*.vue`: không có).
- **ARIA và data:**
  - Root: nhãn "`<nhãn lịch>`, `<chữ trang>`"; nhãn lịch mặc định "Event Date" (`Calendar/useCalendar.ts:354`; `Calendar/CalendarRoot.vue:349`). Heading ẩn cấp 2 cùng chữ (`Calendar/CalendarRoot.vue:364-373`).
  - Chữ trang: 1 tháng "January 1980" (test `Calendar/Calendar.test.ts:78-89`); nhiều tháng cùng năm "January - February 1980"; khác năm "December 1979 - January 1980" (`Calendar/useCalendar.ts:333-351`). Theo hệ lịch của ngày neo; năm trước Công nguyên thêm era dạng ngắn (`Calendar/useCalendar.ts:128-137`).
  - Lưới: role `application`, `aria-readonly`, `aria-disabled` (`Calendar/CalendarGrid.vue:22-25`). Hàng tên thứ `aria-hidden` (`Calendar/CalendarGridHead.vue:16`).
  - Cell: role `gridcell`; `aria-selected` khi được chọn; `aria-disabled` khi bị chặn hoặc không khả dụng (`Calendar/CalendarCell.vue:23-25`).
  - Ô ngày: role `button`; nhãn theo locale gồm thứ, tháng dạng dài, ngày, năm, ví dụ en-US "Sunday, January 20, 1980" (`Calendar/CalendarCellTrigger.vue:57-64,234-235`); `aria-disabled` khi bị chặn hoặc không khả dụng (`:237`). Chữ hiện: số ngày theo chữ số của locale (`:55`).
  - Hôm nay không có trạng thái ARIA, không `aria-current` (suy từ mã, `Calendar/CalendarCellTrigger.vue:229-250`).
  - Prev/Next: nhãn "Previous page" / "Next page"; chữ mặc định "Prev page" / "Next page" (`Calendar/CalendarPrev.vue:40,50`; `Calendar/CalendarNext.vue:42,50`). shadcn-vue giữ nhãn, thay chữ bằng icon lật khi RTL (`shadcn:calendar/CalendarPrevButton.vue:28`, `shadcn:calendar/CalendarNextButton.vue:28`).
  - Look cần đọc: ô — đã chọn, hôm nay, ngoài tháng của lưới, ngoài trang, bị chặn, không khả dụng, là tab stop, số ngày; lịch — disabled, readonly, invalid; Prev/Next — bị chặn; chữ trang; tên thứ; danh sách tháng của trang. shadcn-vue đọc: hôm nay khi chưa chọn, đã chọn, bị chặn, không khả dụng (gạch ngang), ngoài tháng (`shadcn:calendar/CalendarCellTrigger.vue:24-32`).
- **Locale và RTL:**
  - Locale mặc định `en`, hướng mặc định `ltr` (`shared/useLocale.ts:10-14`; `shared/useDirection.ts:11-15`).
  - Ngày đầu tuần mặc định theo locale (`date/calendar.ts:253-261`). Caller cho 0–6, 0 = Chủ nhật tuyệt đối, không tương đối theo locale (`date/comparators.ts:141-185`).
  - Tên thứ, chữ trang, nhãn ô, số ngày theo locale (`Calendar/useCalendar.ts:204-210,326-352`; `Calendar/CalendarCellTrigger.vue:55-64`).
  - RTL: chỉ ArrowLeft/ArrowRight đảo (`Calendar/CalendarCellTrigger.vue:120-127`). Thứ tự cột theo hướng của root (suy từ mã, `Calendar/CalendarRoot.vue:353`).
  - Đổi locale, ngày đầu tuần, 6 tuần, số tháng → dựng lại lưới (`Calendar/useCalendar.ts:316-324`).
- **Cơ chế DOM không port:**
  - Tìm ô theo selector `data-value` rồi `focus()`, chờ `nextTick` sau lật trang → phục vụ: focus đi theo ngày neo qua trang (`Calendar/CalendarCellTrigger.vue:201-224`; `shared/useCalendarKeyboardNavigation.ts:157,170-197`).
  - Giới hạn 24 lần thử cũng che việc chờ lưới render → phục vụ: việc dò luôn dừng (`shared/useCalendarKeyboardNavigation.ts:7-13`).
  - Ô ngoài tháng hoặc bị chặn bỏ `tabindex` khi là `div`, giữ `-1` với thẻ khác → phục vụ: một tab stop dù caller render ô bằng `button` (`shared/useCalendarKeyboardNavigation.ts:15-32`).
  - Gán tạm `tabindex=-1` cho bản ngoài tháng → phục vụ: Home/End tới được ngày tháng kề khi không có bản trong tháng (`shared/useCalendarKeyboardNavigation.ts:194-195`).
  - `preventDefault` + `stopPropagation` trên phím xử lý → phục vụ: mũi tên không cuộn trang, Enter không gửi form (`Calendar/CalendarCellTrigger.vue:116-117,249`).
  - Heading ẩn bằng CSS clip → phục vụ: AT có heading cấp 2 cho lịch (`Calendar/CalendarRoot.vue:364-373`).
  - `disabled` gốc chỉ đặt khi là `button` → phục vụ: nút bị chặn rời thứ tự Tab (`shared/disabledAttr.ts:16-18`).
  - Bảng HTML (`table`, `thead`, `td`) → phục vụ: AT đọc hàng, cột của lưới.
- **Dễ sót:**
  - Tab stop là ngày neo, không phải ngày chọn. Prev/Next đưa ngày neo về ngày 1, nên Tab vào lưới sau khi lật trang tới ngày 1 (`Calendar/useCalendar.ts:228,274`).
  - Mũi tên nhảy qua ngày bị chặn nhưng dừng trên ngày không khả dụng; Home/End/PageUp/PageDown nhảy qua cả hai (`Calendar/CalendarCellTrigger.vue:220-222`; `shared/useCalendarKeyboardNavigation.ts:181`).
  - Mũi tên ra ngoài min/max: đứng yên. PageUp/PageDown: kẹp vào min/max (`Calendar/CalendarCellTrigger.vue:198-199`; `shared/useCalendarKeyboardNavigation.ts:76-80`).
  - Click ô ngoài tháng chọn ngày đó và trang chuyển sang tháng của nó (suy từ mã, `Calendar/CalendarCellTrigger.vue:103-107`; `Calendar/CalendarRoot.vue:255-262`).
  - Nhiều ngày: bỏ một ngày → ngày neo nhảy tới ngày cuối danh sách, có thể đổi trang (suy từ mã, `Calendar/CalendarRoot.vue:255-262`; `date/comparators.ts:228-233`).
  - Hàm lật trang riêng của nút: ngày neo bằng đúng kết quả hàm. Hàm đặt ở root: ngày neo về ngày 1 nếu bước ≥ số ngày của tháng, về tháng 1 nếu bước ≥ 365 ngày (`Calendar/useCalendar.ts:243-255`; test `Calendar/Calendar.test.ts:116-158`).
  - Lỗi reka không chép:
    - Bật "chặn ngày ngoài tháng" làm mọi gridcell `aria-disabled`, cả ngày trong tháng (`Calendar/CalendarCell.vue:25`).
    - Tìm ngày focus được đầu tiên so ngày neo (không phải ngày 1 của tháng) với min: ngày neo trước min trong cùng tháng → bỏ qua cả tháng; lịch một tháng thì mất tab stop (suy từ mã, `Calendar/useCalendar.ts:361-363`).
    - Focus đầu lấy ô khớp đầu tiên theo DOM, có thể là bản ngoài tháng không focus được (suy từ mã, `shared/date/utils.ts:60-62`).
    - Hôm nay tính theo múi giờ máy và không cập nhật qua nửa đêm (suy từ mã, `Calendar/CalendarCellTrigger.vue:69-71`).
    - Nút Prev/Next đang focus bị disabled khi chạm min/max: focus rơi, không chuyển đi (suy từ mã, `Calendar/CalendarNext.vue:27,46`).
    - Enter ở bàn phím số bị nuốt mà không chọn, vì phím được lọc theo `e.code` (suy từ mã, `Calendar/CalendarCellTrigger.vue:116-121,190-192,248`).
    - Nhãn AT cố định tiếng Anh: "Event Date", "Previous page", "Next page".
  - Docs lệch nguồn:
    - Tab "focuses the first navigation button" (`docs:components/calendar.md:303-304`): nguồn không xử lý Tab; câu đó chỉ đúng với bố cục thường (nút trước lưới). Lưới có một tab stop.
    - Mũi tên "changing the month/year/decade" (`docs:components/calendar.md:323-326`): nguồn chỉ lật tháng.
    - Trạng thái focused "Present when focused" (`docs:components/calendar.md:264-265`): nguồn đặt nó trên tab stop, không phải ô đang focus (`Calendar/CalendarCellTrigger.vue:245`).
    - Cấm bỏ chọn "without selecting another date first" (`meta:CalendarRoot.md:140-141`): nguồn chặn mọi lần bỏ.

---

### RangeCalendar
`RangeCalendar/` · dùng bởi: `range-calendar` (`shadcn:range-calendar/`); ví dụ 2 tháng trong popover (`shadcn-ex:calendar/DatePickerWithRange.vue:68-70`), 2 tháng (`shadcn-ex:range-calendar/RangeCalendarMultipleMonths.vue:16`).

- **Part:** như Calendar (`RangeCalendar/RangeCalendarRoot.vue:449-491`). Khác: ô ngày là nút bật/tắt, và ghi "ngày đang trỏ" khi nhận focus hoặc chuột vào (`RangeCalendar/RangeCalendarCellTrigger.vue:313,330-331`); chuột rời lưới thì xoá ngày đang trỏ (`RangeCalendar/RangeCalendarGrid.vue:29`).
- **Trạng thái:**
  - Khoảng: start và end; mỗi đầu có thể rỗng (`RangeCalendar/RangeCalendarRoot.vue:252-255`).
  - Ngày đang trỏ: ô vừa nhận focus hoặc chuột, trừ ô bị chặn hoặc không khả dụng (`RangeCalendar/RangeCalendarCellTrigger.vue:179-183`). Nó quyết định vùng xem trước.
  - Khoảng hợp lệ cuối: khoảng đủ hai đầu gần nhất do người dùng tạo (`RangeCalendar/RangeCalendarRoot.vue:240,372`).
  - Đang chọn dở: bật khi start/end đổi mà chưa đủ hai đầu (`RangeCalendar/RangeCalendarRoot.vue:364-378`).
  - Ngày bấm trước: cho luật "bấm hai lần" (`RangeCalendar/RangeCalendarCellTrigger.vue:121-123,128`).
  - Ngày neo: như Calendar; theo start khi start đổi sang ngày khác (`RangeCalendar/RangeCalendarRoot.vue:342-347`).
  - Caller nhận: khoảng dở (start, end rỗng); khoảng đủ, luôn sắp start ≤ end; khoảng hợp lệ cuối; start mỗi khi đổi; ngày neo (`RangeCalendar/RangeCalendarRoot.vue:242-244,346,365-378`).
  - Caller ghi khoảng: đồng bộ theo ngày, bỏ giờ (`RangeCalendar/RangeCalendarRoot.vue:324-340`). Caller có thể giữ lại một đầu khi người dùng sửa đầu kia (test `RangeCalendar/RangeCalendar.test.ts:182-213`).
  - Tuỳ chọn thêm, mặc định (`RangeCalendar/RangeCalendarRoot.vue:156-175`): khoảng phải liền (không vắt qua ngày không khả dụng); không giới hạn số ngày; không cố định đầu nào. Thêm: hàm "vẫn tô được" (ngày bị chặn hoặc không khả dụng vẫn được nằm trong khoảng).
- **Bàn phím:** như Calendar (`RangeCalendar/RangeCalendarCellTrigger.vue:185-302`). Thêm:

| Phím | Kết quả |
|---|---|
| Escape, khi đang chọn dở | Trả start, end về khoảng hợp lệ cuối (`RangeCalendar/RangeCalendarRoot.vue:381-388`; test `RangeCalendar/RangeCalendar.test.ts:215-247`). Không chặn lan truyền. |
| Escape, khi không chọn dở | Không làm gì; đi lên cha. |
| Phím điều hướng | Dời focus và ngày đang trỏ (vùng xem trước đi theo); không đổi khoảng (test `RangeCalendar/RangeCalendar.test.ts:722-981`). |

- **Chọn** (click, Enter, Space; `RangeCalendar/RangeCalendarCellTrigger.vue:106-171`). Bỏ qua khi readonly, khi ngày bị chặn (kể cả do số ngày tối đa) hoặc không khả dụng. Xét theo thứ tự:
  1. Có start, chưa có end, và không có vùng xem trước (chưa trỏ ô nào, hoặc khoảng tới ngày bấm vướng ngày chặn / không khả dụng):
     - ngày bấm là start → bỏ start, trừ khi cấm bỏ chọn (`:113-118`);
     - ngày khác: lần bấm đầu bị bỏ qua; bấm lại đúng ngày đó → nó thành start mới (`:119-125`).
  2. Khoảng một ngày, bấm đúng ngày đó → xoá cả khoảng, trừ khi cấm bỏ chọn (`:130-141`).
  3. Chưa có start → start (`:143-144`).
  4. Chưa có end → end; end trước start thì hai đầu đổi chỗ khi báo ra (`:146-147`; `RangeCalendar/RangeCalendarRoot.vue:365-372`). Bấm lại start đang được trỏ → khoảng một ngày.
  5. Đủ hai đầu, không cố định → bỏ end, ngày bấm thành start mới (`:149-153`).
  6. Cố định start: bấm trước start → start mới; còn lại → end mới. Cố định end: bấm sau end → end mới; còn lại → start mới (`:154-169`; test `RangeCalendar/RangeCalendar.test.ts:1379-1422`).
  - Chuỗi có test: 5 → 7 (khoảng 5–7) → 8 (start mới) → 8 (khoảng một ngày) → 8 (xoá) (test `RangeCalendar/RangeCalendar.test.ts:121-180`).
- **Vùng xem trước** (`RangeCalendar/useRangeCalendar.ts:107-143`):
  - Chỉ có khi có start, có ngày đang trỏ, và chưa đủ hai đầu (hoặc có cố định đầu).
  - Từ start tới ngày đang trỏ, sắp lại.
  - Có số ngày tối đa N và chưa có end: cửa sổ N ngày từ start về phía ngày đang trỏ, bất kể ngày trỏ ở đâu trong cửa sổ (`:124-133`; test `RangeCalendar/RangeCalendar.test.ts:249-287,1526-1569`).
  - Còn lại: chỉ có nếu mọi ngày sau start tới hết ngày đang trỏ không bị chặn, không khả dụng. Không khả dụng được bỏ qua khi cho khoảng không liền; ngày "vẫn tô được" được miễn; so theo ngày, bỏ giờ (`:135`; `date/comparators.ts:249-274`; test `RangeCalendar/RangeCalendar.test.ts:1706-1735`).
- **Số ngày tối đa N** (`RangeCalendar/useRangeCalendar.ts:74-99`):
  - Chỉ có start: ngày cách start ≥ N ngày (cả hai phía) bị chặn, nên khoảng dài nhất là N ngày tính cả hai đầu (`:91-95`; test `RangeCalendar/RangeCalendar.test.ts:1455-1485,1546-1569`).
  - Đủ hai đầu, không cố định: không chặn gì (test `RangeCalendar/RangeCalendar.test.ts:1487-1524`).
  - Đủ hai đầu, có cố định: chặn ngày làm khoảng mới vượt N ngày (`:80-87`).
- **Focus:** như Calendar, thêm một bậc: ngày neo không focus được → start (nếu không bị chặn) → end → ngày đầu focus được của trang (`RangeCalendar/RangeCalendarCellTrigger.vue:94-104`; `RangeCalendar/useRangeCalendar.ts:179-185`; test `RangeCalendar/RangeCalendar.test.ts:1602-1651`).
- **Giá trị:**
  - Trong khoảng: start, end và mọi ngày giữa (`RangeCalendar/useRangeCalendar.ts:63-72`).
  - Invalid: start hoặc end bị chặn, hoặc end trước start (`RangeCalendar/useRangeCalendar.ts:25-49`). Ngày không khả dụng không làm invalid, khác Calendar (`Calendar/useCalendar.ts:50-72`).
  - Không tham gia form.
- **ARIA và data:**
  - Root, lưới, nhãn ô, Prev/Next như Calendar (`RangeCalendar/RangeCalendarRoot.vue:449-480`).
  - Ô ngày: `aria-pressed` khi trong khoảng, trừ ngày không khả dụng nếu khoảng phải liền (`RangeCalendar/RangeCalendarCellTrigger.vue:313`).
  - Gridcell: `aria-selected` khi trong khoảng, kể cả ngày không khả dụng (`RangeCalendar/RangeCalendarCell.vue:23`).
  - AT không được báo: đầu nào là start, đầu nào là end, vùng xem trước, lần bấm bị bỏ qua (suy từ mã).
  - Look cần đọc: như Calendar, thêm — trong khoảng, là start, là end, trong vùng xem trước, đầu và cuối vùng xem trước; vị trí ô trong hàng (đầu, cuối hàng) để bo góc. shadcn-vue đọc: đã chọn, start, end, đầu/cuối hàng, hôm nay, ngoài tháng, bị chặn, không khả dụng; không đọc vùng xem trước (`shadcn:range-calendar/RangeCalendarCell.vue:18`; `shadcn:range-calendar/RangeCalendarCellTrigger.vue:21-37`).
- **Locale và RTL:** như Calendar (`RangeCalendar/RangeCalendarRoot.vue:224-226`; `RangeCalendar/RangeCalendarCellTrigger.vue:196-202`).
- **Cơ chế DOM không port:**
  - `focusin` + `mouseenter` ghi ngày đang trỏ, `mouseleave` của lưới xoá → phục vụ: vùng xem trước theo cả chuột và bàn phím (`RangeCalendar/RangeCalendarCellTrigger.vue:330-331`; `RangeCalendar/RangeCalendarGrid.vue:29`).
  - Escape nghe trên `keydown` của root, theo `ev.key` → phục vụ: huỷ chọn dở từ bất kỳ đâu trong lịch (`RangeCalendar/RangeCalendarRoot.vue:382-388`).
  - Còn lại như Calendar.
- **Dễ sót:**
  - Bấm lại start khi đang trỏ nó → khoảng một ngày, không phải bỏ chọn; bấm thêm lần nữa → xoá (test `RangeCalendar/RangeCalendar.test.ts:150-162`).
  - Bấm đích vướng ngày chặn: lần đầu không có gì xảy ra; phải bấm lại đúng ngày đó (`RangeCalendar/RangeCalendarCellTrigger.vue:119-125`).
  - Escape chỉ có tác dụng khi đang chọn dở, và không chặn lan truyền (`RangeCalendar/RangeCalendarRoot.vue:382-388`).
  - Vùng xem trước khi có N là cửa sổ N ngày, không phải khoảng sẽ được chọn (`RangeCalendar/useRangeCalendar.ts:124-133`).
  - Cố định một đầu không khoá đầu đó: cố định start mà bấm trước start vẫn dời start (`RangeCalendar/RangeCalendarCellTrigger.vue:154-157`). meta chỉ nói "Which part of the range should be fixed" (`meta:RangeCalendarRoot.md:65-66`).
  - Lỗi reka không chép:
    - Caller ghi khoảng đủ thì khoảng hợp lệ cuối không đổi; Escape sau đó trả về khoảng cũ hơn (suy từ mã, `RangeCalendar/RangeCalendarRoot.vue:349-362`).
    - Có N và chưa có end: vùng xem trước bỏ kiểm tra liền mạch, nên end có thể tạo khoảng vắt qua ngày không khả dụng (suy từ mã, `RangeCalendar/useRangeCalendar.ts:124-135`; `RangeCalendar/RangeCalendarCellTrigger.vue:112`).
    - `aria-selected` và `aria-pressed` lệch nhau ở ngày không khả dụng trong khoảng (`RangeCalendar/RangeCalendarCell.vue:23`; `RangeCalendar/RangeCalendarCellTrigger.vue:313`).
    - Như Calendar: mọi gridcell `aria-disabled` khi chặn ngày ngoài tháng (`RangeCalendar/RangeCalendarCell.vue:24`).
  - Docs lệch nguồn: bảng phím không có Escape (`docs:components/range-calendar.md:294-366`).

---

### Tiện ích ngày dùng chung
`date/`, `shared/date/`, `shared/useCalendarKeyboardNavigation.ts`, `shared/useDateFormatter.ts` · dùng bởi: mọi primitive của nhóm; shadcn-vue gọi thẳng các hàm tạo năm, dải năm, đổi sang `Date`, định dạng cho select tháng/năm (`shadcn:calendar/Calendar.vue:8-9,27-37`).

- **Lưới tháng** (`date/calendar.ts:61-102`):
  - Các ngày của tháng, thêm ngày tháng trước từ đầu tuần và ngày tháng sau tới cuối tuần; chia hàng 7 ngày.
  - Luôn 6 tuần (tuỳ chọn): thêm ngày tháng sau cho đủ 42 ô (`:78-91`; test `Calendar/Calendar.test.ts:355-377`).
  - Ô = ngày neo đặt lại ngày, nên giữ kiểu, giờ, hệ lịch của ngày neo (`:65`).
  - Nhiều tháng: tháng đầu chứa ngày neo, rồi +1, +2… tháng (`:150-184`).
  - Tên thứ lấy từ hàng đầu của tháng đầu (`Calendar/useCalendar.ts:204-210`).
- **Tuần:**
  - Ngày đầu tuần 0–6, 0 = Chủ nhật tuyệt đối (`date/comparators.ts:141-185`; issue 2157 trong comment).
  - Mặc định từ locale: suy ra từ vị trí của Thứ Hai trong tuần của locale (`date/calendar.ts:253-261`).
  - Số tuần: locale kiểu ISO (tuần bắt đầu Thứ Hai) → tuần 1 chứa 4/1; còn lại → tuần 1 chứa 1/1, tuần bắt đầu Chủ nhật. Caller ép được ngày đầu tuần; kiểu ISO vẫn theo locale (`date/calendar.ts:266-290`; test `date/calendar.test.ts:23-75`). Lịch reka không hiện số tuần.
- **Lật trang** (`Calendar/useCalendar.ts:155-186,212-302`):
  - Bước 1 tháng; lật theo cả trang thì bước bằng số tháng (`:216,262`; test `Calendar/Calendar.test.ts:314-353`).
  - Lật mặc định xong: ngày neo = ngày 1 của tháng đầu (`:228,274`).
  - Next bị chặn khi ngày 1 của tháng sau tháng cuối > max; Prev bị chặn khi ngày cuối của tháng trước tháng đầu < min. Không có max / min → không chặn. Lịch disabled → chặn (`:155-186`).
- **So sánh** (`date/comparators.ts`):
  - Trước, sau, giữa: so đầy đủ, có giờ (`:76-139`).
  - "Cùng lựa chọn": lịch so theo ngày; field và picker so chính xác (cùng hệ lịch, cùng chuỗi ISO, cả giờ và múi) (`:193-225`).
  - Liền mạch: mọi ngày sau start tới hết ngày của end, bỏ giờ; ngày "vẫn tô được" được miễn (`:249-274`).
  - Phím điều hướng: so theo ngày; khi kẹp vào min/max, giữ kiểu, giờ, hệ lịch của ô đang focus (`shared/useCalendarKeyboardNavigation.ts:104-129`).
- **Ngày mặc định** (`shared/date/comparators.ts:30-74`): lịch, picker, DateField — hôm nay (theo đồng hồ máy) đổi sang hệ lịch mặc định của locale; DateField có giờ → 00:00:00. TimeField — 00:00:00.
- **AM/PM:** suy từ giờ 0–23 của giá trị (số Latin, theo múi của giá trị), không từ chữ AM/PM của locale (`shared/useDateFormatter.ts:101-133`; test `DateField/DateField.test.ts:1246-1279`, `TimeField/TimeField.test.ts:1434-1446`).
- **Chuỗi form** (`shared/date/utils.ts:89-115`): ngày → `YYYY-MM-DD`; có giờ → `YYYY-MM-DDTHH:mm`, thêm `:ss` khi tới giây; năm đệm 4 chữ số; trường lấy theo hệ lịch của giá trị.
- **Bước** (`shared/date/utils.ts:44-54`): mặc định 1 cho mọi đơn vị; caller ghi đè từng đơn vị.
- **Cơ chế DOM không port:**
  - `@internationalized/date` (kiểu ngày, `set`, `add`, `cycle`, `compare`) và `Intl.DateTimeFormat` → phục vụ: số học lịch và định dạng theo locale. S6 cần nguồn tương đương cho cùng giá trị, không cần API đó.
  - Selector `data-*` trong focus đầu (`shared/date/utils.ts:56-71`) → phục vụ: focus đầu vào ngày chọn hoặc hôm nay.
- **Dễ sót:**
  - Ngày neo có giờ, max chỉ có ngày: ô của ngày max mang giờ của ngày neo nên có thể bị coi là sau max (suy từ mã, phụ thuộc thư viện ngày, `Calendar/useCalendar.ts:191`). Phía min có test giữ đúng ngày min (`Calendar/Calendar.browser.test.ts:26-44`).
  - Lịch phi Gregory: chuỗi form ghi trường của hệ lịch đó, không đổi sang Gregory (`shared/date/utils.ts:99-101`).
  - Hệ lịch mặc định đến từ locale, không cố định Gregory (`shared/date/comparators.ts:46-54`).

---

### TimeField
`TimeField/` · dùng bởi: không component shadcn-vue nào. Ví dụ "Calendar with Time" dùng ô giờ gốc của Chrome (`shadcn-ex:calendar/CalendarWithTime.vue:42,66`), thứ `crates/gui/src/time_input.rs` đang mô phỏng (`crates/gui/src/time_input.rs:1-9`).

- **Part:**
  - Root: nhóm (role `group`) chứa segment theo thứ tự `formatToParts` của locale (`TimeField/TimeFieldRoot.vue:212-221,323-334`; `shared/date/parser.ts:176-216`).
  - Input: một segment — giờ, phút, giây, AM/PM, múi giờ (chỉ hiện). Literal (dấu `:`, khoảng trắng) ẩn với AT (`shared/date/useDateField.ts:214-219`).
  - Input ẩn cho form (`TimeField/TimeFieldRoot.vue:341-352`).
- **Trạng thái:**
  - Giá trị: rỗng hoặc một giờ. Caller có thể đưa giờ thuần, ngày-giờ, ngày-giờ có múi; kiểu được giữ khi ghi lại (`TimeField/TimeFieldRoot.vue:150-165`).
    - Người dùng ghi khi mọi segment có giá trị: sau mỗi phím được nhận (trừ trái, phải, Tab, Shift) và khi rời segment (`shared/date/useDateField.ts:908-918,941-944`).
    - Backspace xoá chữ số duy nhất của một segment → giá trị rỗng ngay (`shared/date/useDateField.ts:315-327`).
    - Caller ghi giá trị → segment đồng bộ lại. Caller ghi rỗng → segment chỉ bị xoá khi đang đủ; segment sửa dở được giữ (`TimeField/TimeFieldRoot.vue:262-270`).
  - Giá trị segment: số hoặc rỗng; AM/PM bắt đầu là `AM`, không bao giờ rỗng (`shared/date/parser.ts:45-61`).
  - Ngày neo: nền để ghép giá trị, và giá trị cho mũi tên đầu tiên trên giờ. Mặc định: giá trị đầu, hoặc 00:00:00 (`shared/date/comparators.ts:62-74`). Theo giá trị khi giá trị đổi (`TimeField/TimeFieldRoot.vue:257-260`).
  - Tuỳ chọn caller (`TimeField/TimeFieldRoot.vue:105-112,136-139,188-193`): độ mịn giờ / phút / giây (mặc định phút); chu kỳ 12 hoặc 24 giờ (mặc định theo locale); bước mỗi đơn vị (mặc định 1); làm tròn theo bước khi rời segment (tắt); min, max; ẩn múi giờ; disabled; readonly; locale; hướng; tên form; required.
- **Bàn phím:** trên segment (`shared/date/useDateField.ts:731-873,881-919`).

| Phím | Kết quả |
|---|---|
| ArrowUp / ArrowDown | ±bước, vòng. Rỗng: giờ lấy giờ của ngày neo; phút, giây lấy 0 (Up) hoặc 59 (Down) (`:302-313,328-340`; test `TimeField/TimeField.test.ts:1082-1133`). |
| ArrowUp / ArrowDown trên giờ, chu kỳ 12 | Vòng 0–23 bên trong; qua 11 ↔ 12 thì AM/PM đổi theo (`:738-749`; test `TimeField/TimeField.test.ts:640-702`). |
| 0–9 | Theo luật "Nhập theo segment". |
| Backspace | Bỏ chữ số cuối; chữ số duy nhất → segment rỗng và giá trị rỗng (`:315-327`; test `TimeField/TimeField.test.ts:1003-1080`). |
| Delete | Không làm gì: không thuộc phím được nhận (`shared/date/segment.ts:16-36`). |
| ArrowLeft / ArrowRight | Segment trước / sau, đảo khi RTL; không vòng (`shared/date/useSegmentNavigation.ts:22-39`; `TimeField/TimeFieldRoot.vue:283-293`). |
| Tab / Shift+Tab | Segment sau / trước: mỗi segment, cả múi giờ, là một tab stop (`shared/date/useDateField.ts:41,227`; test `TimeField/TimeField.test.ts:267-290`). |
| A / P trên AM/PM | Đặt AM / PM, giờ −12 / +12 (`:863-872`). |
| ArrowUp / ArrowDown trên AM/PM | Đảo AM/PM, giờ ±12 (`:852-861`; test `TimeField/TimeField.test.ts:1218-1305`). |
| Enter, Space | Không đổi segment; ghi giá trị nếu đủ. Enter không gửi form. |
| Mọi phím khác, kể cả Ctrl+C, Ctrl+V | Bị chặn khi field sửa được (`shared/date/useDateField.ts:893-894`). Khi disabled hoặc readonly: không chặn, không xử lý (`:889-892`). |

- **Nhập theo segment** (`shared/date/useDateField.ts:439-597,731-789`):
  - Vào segment (lần đầu, hoặc sau khi rời): chữ số đầu thay hẳn giá trị cũ (`TimeField/TimeFieldInput.vue:20,70-73`; `shared/date/useDateField.ts:449-453,529-533`; test `TimeField/TimeField.test.ts:518-531`).
  - Phút, giây (tối đa 59): 0 đầu → 0, ở lại; 6–9 đầu → sang segment sau; chữ số thứ hai → ghép (3, 7 → 37) rồi sang segment sau (`:455-516`; test `TimeField/TimeField.test.ts:1136-1149,1168-1181`).
  - Giờ: tối đa 12 (chu kỳ 12) hoặc 24 (chu kỳ 24); không đặt chu kỳ thì theo locale (`:753-756`).
    - 0 đầu → 0, ở lại. Chữ số đầu > 1 (chu kỳ 12) hoặc > 2 (chu kỳ 24) → sang segment sau (`:535-564`; test `TimeField/TimeField.test.ts:1184-1198`).
    - Ghép vượt tối đa → bắt đầu lại bằng chữ số mới; sang tiếp nếu chữ số đó > 1 / > 2 (`:582-592`; test `TimeField/TimeField.test.ts:1151-1166,1200-1216`: 2, 5 ở chu kỳ 24 → 5 rồi sang phút).
    - Chu kỳ 12: số gõ là giờ hiển thị, đổi sang 0–23 theo AM/PM đang có: 12 AM → 0, 12 PM → 12, n PM → n + 12 (`:758-777`; test `TimeField/TimeField.test.ts:704-864`). Hiển thị 0 → 12, 13–23 → 1–11 (`TimeField/TimeFieldRoot.vue:226-238`; test `TimeField/TimeField.test.ts:554-638`).
  - Tự sang theo thứ tự segment trong DOM (thứ tự locale), kể cả khi RTL (`shared/date/useSegmentNavigation.ts:41-46`; test `TimeField/TimeField.test.ts:60-71`).
  - Gõ liên tục "1" hoặc "0" đi hết các segment (test `TimeField/TimeField.test.ts:440-485`).
  - Làm tròn theo bước (nếu bật), khi rời segment: về bội gần nhất của bước, không vượt tối đa (phút bước 15: 58 → 45) (`shared/date/useDateField.ts:921-939`; `shared/clamp.ts:68-90`; test `TimeField/TimeField.test.ts:866-997`). Làm tròn giờ cập nhật AM/PM (`:925-933`). Mũi tên luôn đi theo bước, bật hay tắt làm tròn (test `TimeField/TimeField.test.ts:954-968`).
  - Segment rỗng hiện `––` (hai dấu U+2013) (`shared/date/placeholders.ts:195`).
  - IME: chữ số trong chuỗi composition được phát lại từng chữ số vào segment đang focus, nên "34" → 3 ở segment này, 4 ở segment sau (`shared/date/useDateField.ts:973-1007`; test `DateField/DateField.test.ts:1086-1097`). Ký tự khác bị bỏ (test `DateField/DateField.test.ts:1023-1032`). Trái/phải không đổi segment khi đang compose (`TimeField/TimeFieldRoot.vue:285-286`).
  - Chỉ nhận chữ số ASCII 0–9 (`shared/date/segment.ts:10-14`; `shared/date/useDateField.ts:13,991-993`).
- **Giá trị:**
  - Chuỗi form = dạng chuỗi của giá trị: giờ thuần `14:30:00` (test `TimeField/TimeField.test.ts:376-399`); ngày-giờ thì có cả ngày (`TimeField/TimeFieldRoot.vue:347`). Input ẩn không có kiểu, min, max: form không kiểm khoảng (`TimeField/TimeFieldRoot.vue:341-352`).
  - Invalid: trước min hoặc sau max. Không có "không khả dụng". Không kẹp (`TimeField/TimeFieldRoot.vue:195-206`).
  - Giá trị = ngày neo đặt mọi trường một lần (`shared/date/useDateField.ts:912-914`).
- **ARIA và data:**
  - Root: role `group`, `aria-disabled` (`TimeField/TimeFieldRoot.vue:327-328`).
  - Segment: role `spinbutton`, `aria-valuemin/max/now/text`; rỗng → `aria-valuetext` "Empty" (`shared/date/useDateField.ts:122-191`).
  - Nhãn segment cố định tiếng Anh: "hour, ", "minute, ", "second, ", "AM/PM", "timezone, " (`:136,160,184,206,224`).
  - Giờ: `aria-valuemin/max` 1–12 (chu kỳ 12) hoặc 0–23 (`:129-130`; test `DateField/DateField.test.ts:1211-1236`); `aria-valuenow` là giờ 0–23; `aria-valuetext` dạng "14 PM" (`:131-132`).
  - AM/PM: `aria-valuetext` "AM" / "PM" (`:201`).
  - Múi giờ: role `textbox`, chỉ đọc, vẫn là tab stop (`:221-230`).
  - Mỗi segment mang `aria-invalid`, `aria-readonly`, `aria-disabled` của field (`TimeField/TimeFieldInput.vue:59-63`).
  - Look cần đọc: segment rỗng (hiện chữ giữ chỗ), segment đang focus, invalid, disabled, readonly; chữ của từng segment; literal giữa các segment.
- **Locale và RTL:**
  - Thứ tự segment và literal theo locale; đổi locale thì thứ tự đổi (`TimeField/TimeFieldRoot.vue:245-255`).
  - Có segment AM/PM khi locale dùng 12 giờ, hoặc caller chọn chu kỳ 12 (test `TimeField/TimeField.test.ts:105-120`).
  - Chữ AM/PM luôn là "AM" / "PM", không dịch (`shared/date/parser.ts:157-164`; test `TimeField/TimeField.test.ts:1434-1446` với `ja-JP`, `zh-CN`, `ko-KR`).
  - Bỏ literal ngoặc vuông của `zh-TW` khi không hiện múi giờ (`shared/date/parser.ts:202-210`; test `TimeField/TimeField.test.ts:121-127`).
  - RTL: ArrowLeft/ArrowRight đảo; tự sang vẫn theo DOM (`shared/date/useSegmentNavigation.ts:16-46`).
- **Cơ chế DOM không port:**
  - `contenteditable`, `inputmode=numeric`, `enterkeyhint`, con trỏ trong suốt → phục vụ: bàn phím ảo kiểu số, không con trỏ nhấp nháy (`shared/date/useDateField.ts:37-48`).
  - Chặn `beforeinput` ngoài composition; chụp và khôi phục node khi IME → phục vụ: chữ chỉ đổi qua luật segment (`:947-986`).
  - Phát lại `keydown` cho từng chữ số IME → phục vụ: IME nhập như gõ phím (`:991-1006`).
  - Input ẩn; focus vào nó chuyển sang segment đầu → phục vụ: nhãn trỏ tới field focus segment đầu; giá trị vào form (`TimeField/TimeFieldRoot.vue:341-352`).
  - `mousedown` bị chặn khi disabled → phục vụ: segment disabled không nhận focus (`shared/date/useDateField.ts:875-879`).
  - Lấy danh sách segment bằng selector khi mount và khi đổi locale → phục vụ: điều hướng theo thứ tự đang hiện (`TimeField/TimeFieldRoot.vue:141-143,245-255`).
- **Dễ sót:**
  - Chữ số đầu sau khi vào segment thay hẳn giá trị; ở lại thì ghép (`TimeField/TimeFieldInput.vue:20,70-73`).
  - Giá trị chỉ ghi khi mọi segment có giá trị; đang dở thì caller vẫn thấy giá trị cũ hoặc rỗng (`shared/date/useDateField.ts:908-918`).
  - Backspace trên segment một chữ số xoá cả giá trị; các segment khác giữ nguyên (`shared/date/useDateField.ts:321-323`).
  - So với `crates/gui/src/time_input.rs` (theo Chrome): chữ số vượt tối đa bị kẹp (2, 5 → 23); Backspace và Delete xoá cả segment (`crates/gui/src/time_input.rs:109-128,143-147`). reka: bắt đầu lại bằng chữ số mới (2, 5 → 5); Backspace xoá từng chữ số; Delete không làm gì. Chrome là tham chiếu verdict cho ô giờ (ví dụ trên dùng ô gốc). S6 chọn một luật và ghi lại.
  - Lỗi reka không chép:
    - Chu kỳ 24: gõ được tới 24 (ngưỡng 24, `shared/date/useDateField.ts:756`) trong khi `aria-valuemax` là 23 (`:130`). Không có test cho 24.
    - Chu kỳ 12: `aria-valuenow` 13–23 vượt `aria-valuemax` 12; `aria-valuetext` "14 PM" (`:128-132`).
    - Giá trị 0 bị coi là rỗng khi tính `aria-valuenow`: giờ, phút, giây bằng 0 đọc ra giá trị của ngày neo (suy từ mã, `:128,150-152,174-176`).
    - Đổi AM/PM khi giờ rỗng ghi giờ 12 hoặc −12 (suy từ mã, `:855,859,865,871`).
    - min/max được so trên ngày hôm nay; giá trị ngày-giờ của ngày khác bị so sai (suy từ mã, `TimeField/TimeFieldRoot.vue:86-92,138-139,195-206`).
    - Không nối với Field (nhãn, mô tả, invalid, disabled, tên, required), khác DateField (`TimeField/TimeFieldRoot.vue:323-352`; so `DateField/DateFieldRoot.vue:127-134,280-288`).
    - Hiển thị dùng định dạng số của locale, có thể là chữ số không phải Latin, trong khi nhập chỉ nhận ASCII; reka chỉ cố định số Latin khi tính AM/PM (suy từ mã, `shared/useDateFormatter.ts:101-133,135-153`).
  - Docs lệch nguồn:
    - Tab "focuses the first segment" (`docs:components/time-field.md:120-121`): mọi segment là tab stop.
    - Backspace "Deletes a digit" (`docs:components/time-field.md:141-142`): xoá chữ số cuối còn xoá cả giá trị.

---

### DateField
`DateField/` · dùng bởi: không component shadcn-vue nào. Là phần gõ tay của DatePicker (`DatePicker/DatePickerField.vue:11-47`).

- **Part:** như TimeField, thêm segment ngày, tháng, năm, era (`shared/date/useDateField.ts:50-120,232-248`). Root role `group` (`DateField/DateFieldRoot.vue:362-374`).
- **Trạng thái:** như TimeField, khác:
  - Giá trị: ngày, ngày-giờ, hoặc ngày-giờ có múi (`DateField/DateFieldRoot.vue:147-150`).
  - Độ mịn: không đặt → ngày nếu ngày neo không có giờ, phút nếu có. Đặt giờ, phút, giây mà ngày neo không có giờ → vẫn là ngày (`DateField/DateFieldRoot.vue:166-171`).
  - Ngày neo mặc định: hôm nay theo hệ lịch của locale; có giờ thì 00:00 (`shared/date/comparators.ts:30-55`). Giá trị đổi (so chính xác cả giờ, múi, hệ lịch) → ngày neo theo (`DateField/DateFieldRoot.vue:219-225`; test `DateField/DateField.test.ts:396-412`).
  - Field: disabled, tên, required lấy từ Field khi field không tự đặt; nhãn và mô tả của Field gộp với của caller; invalid của Field cũng tính (`DateField/DateFieldRoot.vue:127-134,280-288,369`).
  - "Đã sửa" chỉ khi giá trị đổi lúc focus ở trong field; đổi từ code chỉ cập nhật "có giá trị" (`DateField/DateFieldRoot.vue:311-334`).
  - Tuỳ chọn thêm: hàm "không khả dụng"; không có hàm chặn ngày (`DateField/DateFieldRoot.vue:103-110,177`).
- **Bàn phím:** như TimeField, thêm:

| Phím | Kết quả |
|---|---|
| ArrowUp / ArrowDown trên ngày, tháng, năm | ±bước, vòng. Rỗng: lấy trường của ngày neo (`shared/date/useDateField.ts:332-333`). Ngày vòng trong số ngày của tháng đang gõ; tháng rỗng → 31 (`:343-357`; test `DateField/DateField.test.ts:254-270`). |

- **Nhập theo segment:** như TimeField, khác (`shared/date/useDateField.ts:361-437,599-724`):
  - Tháng (tối đa 12) và ngày (tối đa = số ngày của tháng đang gõ; tháng rỗng → 31; năm lấy từ ngày neo) (`:652-656,685`):
    - 0 đầu → segment vẫn rỗng, chờ chữ số sau; chữ số sau ghép và sang tiếp (`:383-386,394-397`).
    - Chữ số đầu lớn hơn chữ số đầu của tối đa → sang tiếp (tháng: 2–9; ngày của tháng 31 ngày: 4–9) (`:394-397`).
    - Ghép vượt tối đa → giữ chữ số mới và sang tiếp (tháng 1, 3 → 3; ngày 3, 3 → 3) (`:422-432`).
  - Năm: 0 đầu → 1; ghép đủ 4 chữ số thì sang tiếp; chữ số thứ năm bắt đầu lại (0 → 1); khoảng 1–9999 (`:599-626,106-107`; test `DateField/DateField.test.ts:1147-1188`).
  - Gõ liên tục "3": tháng 3 → ngày 3, 3 (thành 3) → năm 3333 → giờ 3 → phút 33 → giây 33 → AM/PM (test `DateField/DateField.test.ts:568-597`).
  - Gõ 0 không tự sang; rời đi rồi quay lại gõ 1 thì ở lại (test `DateField/DateField.test.ts:323-343`).
  - Chữ giữ chỗ ngày/tháng/năm theo ngôn ngữ: `en` yyyy/mm/dd, `de` jjjj/mm/tt, `fr` aaaa/mm/jj, `ja` 年/月/日; locale không có → ngôn ngữ của nó → `en` (`shared/date/placeholders.ts:89-179`). Không có `vi` (`shared/date/placeholders.ts:5-81`): tiếng Việt hiện yyyy/mm/dd (suy từ mã).
  - Xoá một segment không xoá segment khác (test `DateField/DateField.test.ts:345-372`). Caller ghi rỗng → xoá hết segment (test `DateField/DateField.test.ts:374-394`).
- **Giá trị:**
  - Invalid: không khả dụng, trước min, sau max (so cả giờ) (`DateField/DateFieldRoot.vue:173-187`). Không kẹp. Mọi segment mang `aria-invalid` (test `DateField/DateField.test.ts:455-492`).
  - Form: input ẩn kiểu ngày hoặc ngày-giờ; giá trị `YYYY-MM-DD` hoặc `YYYY-MM-DDTHH:mm[:ss]`; có min, max, required; bước 1 giây khi tới giây (`DateField/DateFieldRoot.vue:246-249,381-397`). Ràng buộc gốc (thiếu, ngoài khoảng) do input đó báo (`DateField/DateFieldRoot.vue:294-295`). Ngày không khả dụng không chặn gửi (suy từ mã).
  - Form báo lỗi → focus segment đầu (`DateField/DateFieldRoot.vue:300-307`).
- **ARIA và data:** như TimeField, thêm:
  - Root: `aria-labelledby`, `aria-describedby` gộp với Field; `aria-invalid` (`DateField/DateFieldRoot.vue:280-288,363`).
  - Ngày: nhãn "day,", 1 tới số ngày của tháng đang gõ (`shared/date/useDateField.ts:50-77`).
  - Tháng: nhãn "month, ", 1–12, `aria-valuetext` dạng "1 - January" (`:79-100`).
  - Năm: nhãn "year, ", 1–9999 (`:102-120`).
  - Look cần đọc: như TimeField.
- **Locale và RTL:** như TimeField. Thứ tự ngày, tháng, năm theo locale (test `DateField/DateField.test.ts:128-141`). RTL: tự sang theo DOM (test `DateField/DateField.test.ts:494-509`).
- **Cơ chế DOM không port:** như TimeField, thêm: input ẩn mang id của Field để nhãn trỏ tới (`DateField/DateFieldRoot.vue:290-292`); đăng ký control với Field để form focus vào segment đầu (`DateField/DateFieldRoot.vue:298-309`).
- **Dễ sót:**
  - Độ mịn bị ép về ngày khi ngày neo không có giờ (`DateField/DateFieldRoot.vue:166-171`).
  - DateField không có hàm chặn: trong DatePicker, ngày bị chặn ở lịch vẫn gõ được và không invalid (`DatePicker/DatePickerField.vue:17-32`; `DateField/DateFieldRoot.vue:173-187`).
  - Lỗi reka không chép:
    - Ngày tối đa lấy năm của ngày neo, không phải năm đang gõ: thứ tự tháng-ngày-năm (en-US) với năm neo không nhuận thì không gõ được 29/2 (suy từ mã, `shared/date/useDateField.ts:652-653`).
    - Chu kỳ 12 định dạng bằng `h11` (0–11) và không đổi 0 → 12 như TimeField: nửa đêm có thể hiện "0" (suy từ mã, `shared/date/utils.ts:73-79`; `shared/date/parser.ts:121-125`; so `TimeField/TimeFieldRoot.vue:226-238`).
    - Era là `spinbutton` không có xử lý phím; phím trên nó gọi một handler không tồn tại (suy từ mã, `shared/date/useDateField.ts:232-248,895-906`).
    - Chuỗi form lấy trường theo hệ lịch của giá trị (`shared/date/utils.ts:99-101`).
    - Các lỗi ARIA giờ của TimeField (cùng code).
  - Docs lệch nguồn: Tab, Backspace như TimeField (`docs:components/date-field.md:121-122,142-143`). Độ mịn "Defaults to day if a CalendarDate is provided" (`meta:DateFieldRoot.md:44-45`) không nói việc ép về ngày.

---

### DatePicker
`DatePicker/` · dùng bởi: không component shadcn-vue nào; shadcn-vue tự ghép Popover + nút + Calendar (`shadcn-ex:calendar/DatePickerSimple.vue:35-57`; `shadcn-ex:calendar/DatePickerWithDropdowns.vue:36-72`).

Thêm gì so với DateField + Calendar + Popover:
- Một giá trị và một ngày neo chung cho field và lịch (`DatePicker/DatePickerRoot.vue:118-140`; `DatePicker/DatePickerField.vue:11-47`; `DatePicker/DatePickerCalendar.vue:12-52`).
- Mở/đóng là của Popover; mặc định đóng, không modal (`DatePicker/DatePickerRoot.vue:71-87,137-140,223-229`). Escape, click ngoài, trả focus: theo Popover (nhóm overlay).
- Đóng khi chọn (tuỳ chọn, mặc định tắt): chỉ khi ngày đổi; đổi giờ hay múi trong cùng ngày thì giữ mở (`DatePicker/DatePickerRoot.vue:166-170`; test `DatePicker/DatePicker.test.ts:391-425,507-521`).
- Giờ: chọn ngày trên lịch giữ giờ của giá trị (ô mang giờ của ngày neo). Xoá giá trị → giờ ngày neo về 00:00, lần chọn sau bắt đầu từ nửa đêm (`DatePicker/DatePickerRoot.vue:147-165`; test `DatePicker/DatePicker.test.ts:269-335`). Giờ gõ tay sau khi xoá được giữ (test `DatePicker/DatePicker.test.ts:336-389`).
- Bấm lại ngày đang chọn trên lịch → rỗng, trừ khi cấm bỏ chọn (test `DatePicker/DatePicker.test.ts:256-267`). Khi cấm bỏ chọn, lịch gửi lại cùng ngày và bị bỏ qua, nên giờ không bị ghi đè (`DatePicker/DatePickerCalendar.vue:35,39`).
- Nút mở nằm cuối chuỗi mũi tên của field (`DatePicker/DatePickerTrigger.vue:18-24`; test `DatePicker/DatePicker.test.ts:193-217`). Tab đi qua từng segment rồi tới nút (test `DatePicker/DatePicker.test.ts:219-242`).
- Mở: focus vào lịch — ngày đã chọn → hôm nay → ngày đầu không bị chặn (`DatePicker/DatePickerContent.vue:31-36`).
- Field chỉ nhận "không khả dụng" và min/max; hàm chặn ngày chỉ áp cho lịch (`DatePicker/DatePickerField.vue:17-32`; `DatePicker/DatePickerCalendar.vue:15-30`).
- Disabled: không mở được, không sửa được (test `DatePicker/DatePicker.test.ts:244-254`).
- Look cần đọc: mở / đóng; cộng mọi trạng thái của field và của lịch.

---

### DateRangePicker
`DateRangePicker/` · dùng bởi: không component shadcn-vue nào; shadcn-vue ghép Popover + RangeCalendar 2 tháng (`shadcn-ex:calendar/DatePickerWithRange.vue:48-73`).

Thêm gì so với DatePicker:
- Field khoảng (hai bộ segment: start, end) và RangeCalendar (`DateRangePicker/DateRangePickerField.vue`; `DateRangePicker/DateRangePickerCalendar.vue:12-57`).
- Ngày neo theo start (`DateRangePicker/DateRangePickerRoot.vue:157-160`).
- Đóng khi chọn: khi có đủ start và end, kể cả khi caller ghi khoảng đủ (`DateRangePicker/DateRangePickerRoot.vue:162-166`; test `DateRangePicker/DateRangePicker.test.ts:212-250`).
- Mũi tên đi qua segment của start, rồi end, rồi nút mở; RTL đảo (test `DateRangePicker/DateRangePicker.test.ts:125-189`).
- Invalid của field khoảng: một đầu không khả dụng hoặc ngoài min/max; start sau end; có ngày không khả dụng ở giữa (`DateRangeField/DateRangeFieldRoot.vue:166-219`).
- Form: một input ẩn "`<start>` - `<end>`" (chuỗi của hai giá trị); rỗng khi thiếu một đầu, nên required báo thiếu (`DateRangeField/DateRangeFieldRoot.vue:276-281,389-400`).
- Escape khi đang chọn dở: lịch trả về khoảng hợp lệ cuối, và vì không chặn lan truyền, lớp cha (popover) cũng nhận Escape (suy từ mã, `RangeCalendar/RangeCalendarRoot.vue:382-388`).
- Look cần đọc: như DatePicker, thêm trạng thái khoảng của lịch.

---

### Ghi chú chung cho nhóm 7
**Hai lõi.** Lõi lưới (Calendar, RangeCalendar, phần lịch của hai picker) và lõi segment (TimeField, DateField, DateRangeField, phần field của hai picker). Mỗi lõi port một lần; picker chỉ ghép.

**Lõi lưới:**
- Trạng thái: giá trị + ngày neo. Ngày neo quyết định trang và tab stop. Phím điều hướng chỉ ghi ngày neo; chọn chỉ qua click, Enter, Space.
- Lưới: N tháng liên tiếp từ tháng của ngày neo; 7 cột; 4–6 hàng, hoặc luôn 6; ô giữ kiểu và giờ của ngày neo.
- Ba loại ngày đặc biệt:

| Loại | Focus được | Tab stop | Mũi tên | Home/End/PageUp/PageDown | Chọn được |
|---|---|---|---|---|---|
| Bị chặn | không | không | nhảy qua | nhảy qua | không |
| Không khả dụng | có | không | dừng trên nó | nhảy qua | không |
| Ngoài tháng | không; focus vào bản trong tháng, đổi trang nếu cần | không | không dừng | tới được | có, trừ khi caller chặn ngày ngoài tháng |

- Luật phím: ±1 ngày, ±7 ngày, đầu/cuối tuần, ±1 tháng, ±1 năm (giữ ngày, kẹp ngày cuối tháng, kẹp min/max); dò lối ra tối đa 24 bước; tự lật trang.
- AT: lưới `application`; ô `button` có nhãn đầy đủ; gridcell `aria-selected` / `aria-disabled`; root có nhãn = nhãn lịch + chữ trang; heading ẩn cấp 2. Không có live region: đổi trang chỉ được đọc khi focus đổi.

**Lõi segment:**
- Trạng thái: bảng giá trị segment (rỗng hoặc số; AM/PM luôn có) + giá trị + ngày neo. Giá trị = ngày neo đặt mọi segment một lần, chỉ khi không segment nào rỗng.
- Thứ tự segment và literal: `formatToParts` của locale (`shared/date/utils.ts:5-38`; `shared/date/parser.ts:176-216`).
- Luật gõ: chữ số đầu sau khi vào thay giá trị; tự sang khi không chữ số nào có thể theo sau; ghép vượt tối đa → bắt đầu lại bằng chữ số mới; 0 đầu chờ chữ số thứ hai.

| Segment | Tối đa khi gõ | Chữ số đầu tự sang | 0 đầu | Nguồn (`shared/date/useDateField.ts`) |
|---|---|---|---|---|
| Tháng | 12 | 2–9 | rỗng, chờ | `:383-397,685` |
| Ngày | số ngày của tháng đang gõ (31 khi chưa có tháng) | lớn hơn chữ số đầu của tối đa | rỗng, chờ | `:383-397,652-658` |
| Năm | 4 chữ số | không; sang ở chữ số thứ 4 | thành 1 | `:599-626` |
| Giờ, chu kỳ 12 | 12 | 2–9 | 0, chờ | `:542-556,753-756` |
| Giờ, chu kỳ 24 | 24 | 3–9 | 0, chờ | `:542-556,753-756` |
| Phút, giây | 59 | 6–9 | 0, chờ | `:439-517` |

- Backspace bỏ chữ số cuối; hết chữ số → segment rỗng và giá trị rỗng. Mũi tên lên/xuống ±bước, vòng; rỗng → trường của ngày neo (phút, giây: biên). Trái/phải theo hướng; Tab qua từng segment; tự sang theo DOM.

**Ghép picker:** field + lịch + popover dùng chung giá trị và ngày neo. Picker thêm: đóng khi chọn (ngày đổi; khoảng đủ), đưa giờ ngày neo về 00:00 khi xoá, nút mở trong chuỗi mũi tên, focus vào lịch khi mở.

**Chuỗi AT cố định tiếng Anh** trong reka: "Event Date", "Previous page", "Next page", "day,", "month, ", "year, ", "hour, ", "minute, ", "second, ", "AM/PM", "timezone, ", "era", "Empty" (`Calendar/useCalendar.ts:354`; `Calendar/CalendarPrev.vue:40`; `Calendar/CalendarNext.vue:42`; `shared/date/useDateField.ts:66,70,92,113,136,160,184,206,224,242`). S6 cần chúng qua bản địa hoá.

**S6 phải quyết và ghi lại** (reka không giải):
- Dịch AM/PM và nhãn AT theo locale.
- Chữ giữ chỗ tiếng Việt.
- Nhận chữ số không phải ASCII.
- Có thông báo cho AT khi đổi trang, khi một lần bấm range bị bỏ qua, hay không.
- Focus đi đâu khi nút Prev/Next đang focus bị chặn.
- Luật gõ giờ: kẹp như Chrome (`crates/gui/src/time_input.rs:124`) hay bắt đầu lại như reka.
- Sửa các lỗi reka liệt kê ở từng mục "Dễ sót"; khác biệt verdict thấy thì ghi vào `verdict/better.tsv`.

**Verdict:**
- shadcn-vue chỉ dùng Calendar và RangeCalendar; ô giờ trong ví dụ là ô gốc của Chrome.
- Ví dụ Week Numbers và Multiple truyền tham số mà wrapper không đọc (`show-week-number`, `type="multiple"`), nên Chrome không hiện số tuần và không chọn nhiều ngày (`shadcn-ex:calendar/CalendarWeekNumbers.vue:22-24`; `shadcn-ex:calendar/CalendarMultiple.vue:11`; `shadcn:calendar/Calendar.vue:14`).
- Ví dụ Booked Dates đưa danh sách ngày vào cờ disabled của cả lịch, nên cả lịch disabled trong Chrome (`shadcn-ex:calendar/CalendarBookedDates.vue:30-34`).

## 12. Nhóm 8: cấu trúc

### Accordion

`Accordion/` · dùng bởi: shadcn `accordion` (Root, Item, Header bọc Trigger, Content)

**Part:**
- Root: nhóm item, sở hữu tập item mở.
- Item: một tiêu đề + một nội dung. Là một Collapsible mà Root quyết định mở/đóng (`Accordion/AccordionItem.vue:115-125`).
- Header: tiêu đề bọc trigger (`Accordion/AccordionHeader.vue:13-15`).
- Trigger: nút bật/tắt item (`Collapsible/CollapsibleTrigger.vue:12-14`, `:22`).
- Content: nội dung của item.

**Model:**
- Sở hữu: tập giá trị item đang mở.
  - single: một giá trị hoặc rỗng. Mặc định rỗng (`shared/useSingleOrMultipleValue.ts:35-40`).
  - multiple: danh sách. Mặc định rỗng (`:35-40`).
  - Caller chỉ định chế độ. Không chỉ định thì suy từ giá trị: mảng → multiple, còn lại → single (`:18-33`).
- Người dùng ghi: kích hoạt trigger (`:53-67`).
  - single: item khác → mở nó, đóng item cũ. Item đang mở → đóng, chỉ khi caller cho đóng hết (`Accordion/AccordionTrigger.vue:20-26`).
  - multiple: đang mở → bỏ khỏi danh sách. Đang đóng → thêm vào cuối danh sách.
  - Item disabled: không ghi (`Accordion/AccordionTrigger.vue:20-26`).
- Caller ghi: đặt cả tập (controlled) hoặc chỉ giá trị đầu. Mỗi thay đổi báo caller giá trị mới: một giá trị, danh sách, hoặc rỗng (`Accordion/AccordionRoot.vue:46-51`).
- Caller cấu hình, mặc định (`Accordion/AccordionRoot.vue:74-79`): disabled cả nhóm (tắt); hướng (dọc); cho đóng hết trong single (không); giữ nội dung khi đóng (không). Item ghi đè được "giữ nội dung" (`Accordion/AccordionItem.vue:46-51`, `:123`).
- Item mở = (single) bằng giá trị, (multiple) có trong danh sách (`Accordion/AccordionItem.vue:62-67`). Item disabled = nhóm disabled hoặc item disabled (`:69-71`).

**Bàn phím:** (focus trên trigger)

| Phím | Kết quả |
|---|---|
| `Enter`, `Space` | Như click trigger (nút native). |
| `Tab`, `Shift+Tab` | Mỗi trigger enabled là một tab stop. Nội dung mở nằm giữa, theo thứ tự cây. |
| `ArrowDown` (dọc), `ArrowRight` (ngang, LTR) | Focus trigger kế, bỏ qua trigger disabled, vòng về đầu (`shared/useArrowNavigation.ts:89`, `:128`, `:194-204`). |
| `ArrowUp` (dọc), `ArrowLeft` (ngang, LTR) | Focus trigger trước, bỏ qua disabled, vòng về cuối. |
| `Home`, `End` | Nhắm trigger đầu / cuối, không bỏ qua disabled (`shared/useArrowNavigation.ts:134-139`). |
| Mũi tên khác trục | Không xử lý, trang vẫn cuộn (`shared/useArrowNavigation.ts:105-113`). |

- Mũi tên chỉ dời focus, không mở item (`Accordion/AccordionItem.vue:101-110`).
- Phím đã xử lý thì chặn cuộn trang (`shared/useArrowNavigation.ts:91`, `:122-123`).

**Pointer:**
- Click trigger: bật/tắt.
- Không tác dụng, không báo caller khi: item disabled; hoặc single + item đang mở + không cho đóng hết (`Accordion/AccordionTrigger.vue:20-26`; `Accordion/Accordion.test.ts:159-169`).

**Trạng thái:**
- Mỗi item: mở / đóng; disabled.
- Đóng, mặc định: nội dung rời cây. AT và find-in-page không thấy (`Collapsible/CollapsibleContent.vue:136`, `:144`).
- Đóng, có giữ nội dung: ẩn nhưng find-in-page tìm thấy. Trúng thì item mở qua model và báo caller (`Accordion/AccordionContent.vue:35`; `Collapsible/CollapsibleContent.vue:115-120`; docs `accordion.md:333-337`).
- Đang đóng: nội dung còn hiện tới hết animation đóng (`Presence/usePresence.ts:15`, `:193`).
- Mở sẵn lúc mount: không chạy animation mở (`Collapsible/CollapsibleContent.vue:90-96`).

**ARIA và data:**
- Trigger: nút; `aria-expanded` = mở (`Accordion/AccordionTrigger.vue:37`); `aria-controls` → content (`Collapsible/CollapsibleTrigger.vue:25`).
- Trigger disabled: `aria-disabled` + `disabled` native (`Accordion/AccordionTrigger.vue:36`, `:41`).
- Item mở mà không được đóng: không có `aria-disabled`, vẫn focus được (`:36`).
- Header: heading cấp 3 (`h3`) (`Accordion/AccordionHeader.vue:13-15`).
- Content: `role=region`, `aria-labelledby` → trigger (`Accordion/AccordionContent.vue:23`, `:27`).
- Look cần đọc: mở/đóng của item (cho trigger, header, content); disabled; hướng; kích thước tự nhiên của content cho animation mở/đóng; cờ mở-sẵn-lúc-mount để bỏ animation.

**RTL:**
- Chỉ trục ngang đổi: `ArrowLeft` = kế, `ArrowRight` = trước (`shared/useArrowNavigation.ts:128`).
- Hướng: tùy chọn của Root, rồi cấu hình chung, mặc định `ltr` (`shared/useDirection.ts:11-16`).

**Cơ chế DOM không port:**
- Query `[data-reka-collection-item]` theo thứ tự tài liệu (`Accordion/AccordionItem.vue:95`) → thứ tự trigger cho mũi tên.
- `hidden=until-found` + `beforematch` (`Collapsible/CollapsibleContent.vue:115-120`, `:136`) → find-in-page mở item.
- Đo khi chặn animation, biến CSS kích thước (`Collapsible/CollapsibleContent.vue:53-88`; `Accordion/AccordionContent.vue:31-34`) → animation theo kích thước thật.
- Không đo nội dung chưa từng hiện (`Collapsible/CollapsibleContent.vue:49-51`; `Accordion/Accordion.test.ts:292-349`) → item đóng lúc mount không tốn layout.
- Trạng thái chưa xác định tới frame đầu (`Collapsible/CollapsibleContent.vue:90-96`) → không animate lúc mount.
- Xóa animation sau `animationend` (`:98-113`) → không chạy lại khi node bị dời trong DOM.

**Dễ sót:**
- Single, không cho đóng hết: kích hoạt item đang mở không làm gì, không báo caller (`Accordion/Accordion.test.ts:159-169`).
- `Home`/`End` không bỏ qua disabled, mũi tên thì có. Trong Chrome focus đứng yên vì nút `disabled` không nhận focus. Quirk: S6 nên bỏ qua disabled ở cả hai (`shared/useArrowNavigation.ts:134-139`).
- Vòng mũi tên gồm mọi collection item của reka dưới Root. Accordion lồng nhau, tab hay menu item trong content mở chung một vòng phẳng (`Accordion/AccordionItem.vue:95-99`; `Collection/Collection.ts:10`). Quirk: S6 chỉ vòng qua trigger của chính accordion.
- "Cho đóng hết" và hướng chỉ đọc lúc setup; đổi sau không có tác dụng (`Accordion/AccordionRoot.vue:100`, `:103`). Quirk.
- multiple: item mới mở nằm cuối danh sách, không theo thứ tự item (`shared/useSingleOrMultipleValue.ts:53-67`).
- Không chỉ định chế độ mà đưa mảng → multiple (`:18-33`).

### Collapsible

`Collapsible/` · dùng bởi: shadcn `collapsible`; shadcn `trace` qua `collapsible` (shadcn: `trace/TraceCollapsible.vue`)

**Part:**
- Root: sở hữu trạng thái mở.
- Trigger: nút bật/tắt (`Collapsible/CollapsibleTrigger.vue:12-14`, `:22`, `:30`).
- Content: phần ẩn/hiện.

**Model:**
- Sở hữu: mở (bool). Mặc định đóng (`Collapsible/CollapsibleRoot.vue:40-44`).
- Người dùng ghi: trigger đảo trạng thái. Bị bỏ qua khi disabled (`:67-72`).
- Người dùng ghi gián tiếp: find-in-page trúng nội dung ẩn → mở, báo caller "đã tìm thấy nội dung" (`Collapsible/CollapsibleContent.vue:115-120`).
- Caller ghi: đặt mở/đóng. Vẫn có tác dụng khi disabled (`Collapsible/CollapsibleRoot.vue:67-72`).
- Mỗi thay đổi báo caller (`:18-21`).
- Caller cấu hình: disabled; giữ nội dung khi đóng (mặc định không) (`:40-44`).

**Bàn phím:**

| Phím | Kết quả |
|---|---|
| `Enter`, `Space` trên trigger | Bật/tắt (nút native) (docs `collapsible.md:201-212`). |
| `Tab` | Trigger là tab stop; disabled thì không (`Collapsible/CollapsibleTrigger.vue:29`). |

**Pointer:**
- Click trigger: bật/tắt (`Collapsible/CollapsibleTrigger.vue:30`).

**Trạng thái:**
- Mở / đóng; disabled.
- Đóng, mặc định: nội dung rời cây (`Collapsible/CollapsibleContent.vue:144`).
- Đóng, có giữ nội dung: còn trong cây, ẩn, find-in-page tìm thấy (`:136`).
- Đang đóng: nội dung còn hiện tới hết animation (`:124-129`; `Presence/usePresence.ts:15`, `:193`).
- Mở sẵn lúc mount: không animate (`Collapsible/CollapsibleContent.vue:90-96`, `:137`).

**ARIA và data:**
- Trigger: `aria-expanded` (`Collapsible/CollapsibleTrigger.vue:26`); `aria-controls` → content (`:25`); disabled: `disabled` native (`:29`).
- Content: không role (chỉ Accordion thêm `region`); có id cho trigger trỏ tới (`Collapsible/CollapsibleContent.vue:35`).
- Look cần đọc: mở/đóng; disabled; kích thước tự nhiên của content cho animation; cờ mở-sẵn-lúc-mount.

**RTL:**
- Không ảnh hưởng.

**Cơ chế DOM không port:**
- `Presence` luôn mount, bao content (`Collapsible/CollapsibleContent.vue:124-129`) → content còn hiện suốt animation đóng.
- Đo khi chặn animation, biến CSS kích thước (`:53-88`, `:139-142`) → animation theo kích thước thật.
- Không đo khi chưa từng hiện (`:49-51`, `:67-69`) → content đóng lúc mount không tốn layout.
- Trạng thái chưa xác định tới frame đầu (`:90-96`) → không animate lúc mount.
- `animationName: none` sau `animationend` (`:98-113`) → không chạy lại animation khi node bị dời.
- `hidden=until-found` + `beforematch`, mở ở frame sau (`:115-120`, `:136`) → find-in-page mở content.

**Dễ sót:**
- Caller ghi được cả khi disabled; chỉ người dùng bị chặn (`Collapsible/CollapsibleRoot.vue:67-72`).
- Find-in-page chỉ thấy content đóng khi caller chọn giữ nội dung (`Collapsible/CollapsibleContent.vue:136`, `:144`).
- Mở do find-in-page đi qua model và báo caller, không chỉ hiện tạm (`:115-120`).

### Tabs

`Tabs/` · dùng bởi: shadcn `tabs` (Root, List, Trigger, Content; không dùng Indicator)

**Part:**
- Root: sở hữu tab đang chọn.
- List: `tablist`, một tab stop (roving).
- Trigger: một tab.
- Content: panel của một giá trị.
- Indicator: vạch chỉ tab đang chọn, đặt theo kích thước tab.

**Model:**
- Sở hữu: một giá trị đang chọn (`Tabs/TabsRoot.vue:86-89`, `:98-100`).
- Người dùng ghi: chọn tab bằng nhấn chuột trái, `Enter`/`Space`, hoặc focus ở chế độ tự động (`Tabs/TabsTrigger.vue:56-78`). Tab disabled: không ghi.
- Caller ghi: đặt giá trị (controlled) hoặc giá trị đầu. Mỗi thay đổi báo caller.
- Caller cấu hình, mặc định (`Tabs/TabsRoot.vue:68-72`; `Tabs/TabsList.vue:17-19`): hướng (ngang); kích hoạt (tự động: focus là chọn; thủ công: cần `Enter`/`Space`/nhấn); vòng mũi tên (có); giữ panel ẩn (không); `dir`.
- Chế độ kích hoạt chỉ đọc lúc setup (`Tabs/TabsRoot.vue:104`).

**Bàn phím:**

| Phím | Kết quả |
|---|---|
| `Tab` vào list | Focus tab đang chọn; không có thì tab stop hiện tại, rồi tab enabled đầu (`RovingFocus/RovingFocusGroup.vue:85-116`). |
| `Tab` từ tab | Rời list, tới phần tử kế, thường là panel (panel là tab stop) (`Tabs/TabsContent.vue:68`). |
| `Shift+Tab` từ tab | Rời list ngay, không dừng ở tab khác (`RovingFocus/RovingFocusItem.vue:65-68`). |
| `ArrowRight` (ngang), `ArrowDown` (dọc) | Tab enabled kế. Vòng nếu bật vòng, không thì dừng ở cuối (`RovingFocus/utils.ts:9-18`; `RovingFocus/RovingFocusItem.vue:83`, `:95-97`). |
| `ArrowLeft` (ngang), `ArrowUp` (dọc) | Tab enabled trước. |
| `Home`, `PageUp` | Tab enabled đầu (`RovingFocus/RovingFocusItem.vue:83-87`). |
| `End`, `PageDown` | Tab enabled cuối. |
| Mũi tên khác trục | Bỏ qua (`RovingFocus/utils.ts:32-43`). |
| Phím điều hướng kèm `Ctrl`/`Alt`/`Meta`/`Shift` | Bỏ qua (`RovingFocus/RovingFocusItem.vue:80-81`). |
| `Enter`, `Space` | Chọn tab đang focus (`Tabs/TabsTrigger.vue:68-70`). |

- Chế độ tự động: mũi tên dời focus nên cũng đổi tab chọn (`Tabs/TabsTrigger.vue:71-78`).

**Pointer:**
- Nhấn (mousedown) chuột trái, không `Ctrl`: chọn ngay lúc nhấn, không đợi thả (`Tabs/TabsTrigger.vue:56-67`).
- Chuột phải, chuột giữa, `Ctrl`+nhấn, tab disabled: không focus, không chọn (`:56-67`; `Tabs/Tabs.browser.test.ts:34-43`; `Tabs/Tabs.test.ts:340-347`).

**Trạng thái:**
- Tab: chọn / không; disabled.
- Panel: active / inactive. Inactive, mặc định: rời cây (`Tabs/TabsContent.vue:73`). Có giữ: ẩn hẳn, find-in-page không thấy (`:67`).
- Panel đang rời: còn hiện tới hết animation (`:53-57`).
- Panel active lúc mount: không animate (`:33-40`, `:69-71`).
- Indicator: chỉ hiện khi đã đo được tab chọn (`Tabs/TabsIndicator.vue:70`). Đo lại khi đổi tab chọn, đổi `dir`, khi list hoặc tab đổi kích thước (`:35-43`).
  - Ngang: độ dài = rộng tab, độ dày = cao tab, vị trí = cách mép trái list.
  - Dọc: độ dài = cao tab, độ dày = rộng tab, vị trí = cách mép trên (`:45-65`).
- List không tab nào enabled: không là tab stop (`RovingFocus/RovingFocusGroup.vue:152`).

**ARIA và data:**
- List: `role=tablist`, `aria-orientation` (`Tabs/TabsList.vue:37`, `:40`).
- Tab: `role=tab` (`Tabs/TabsTrigger.vue:46`); `aria-selected` `"true"`/`"false"` (`:50`); disabled: `disabled` native (`:53`).
- Tab: `aria-controls` → panel, chỉ khi panel cùng giá trị đang tồn tại (`:30-32`; `Tabs/Tabs.test.ts:185-266`).
- Một tab stop: tab đang giữ lượt có `tabindex=0`, còn lại `-1` (`RovingFocus/RovingFocusItem.vue:126`).
- Panel: `role=tabpanel`, `aria-labelledby` → tab, luôn `tabindex=0` (`Tabs/TabsContent.vue:63`, `:66`, `:68`).
- Look cần đọc: chọn/không của tab; disabled; hướng; panel active; indicator: vị trí, độ dài, độ dày của tab chọn trên trục của list.

**RTL:**
- Trục ngang: `ArrowLeft` = kế, `ArrowRight` = trước (`RovingFocus/utils.ts:20-28`).
- Root áp `dir` cho cả cây con (`Tabs/TabsRoot.vue:123`).
- Indicator đo lại khi `dir` đổi (`Tabs/TabsIndicator.vue:35-37`).

**Cơ chế DOM không port:**
- `tabindex` trên container + cờ "đang Shift+Tab ra" (`RovingFocus/RovingFocusGroup.vue:85-116`, `:152`, `:161`) → một tab stop, `Shift+Tab` thoát list.
- `preventDefault` trên `mousedown` (`Tabs/TabsTrigger.vue:63-66`) → nhấn sai nút không lấy focus.
- `ResizeObserver` + `offsetLeft`/`offsetWidth` (`Tabs/TabsIndicator.vue:39-65`) → indicator bám tab chọn.
- `animation-duration: 0s` lúc mount (`Tabs/TabsContent.vue:69-71`) → không animate lúc mount.
- Panel đăng ký khi mount (`:33-40`) → `aria-controls` không trỏ vào id không tồn tại.

**Dễ sót:**
- Chọn lúc nhấn (mousedown), không lúc click (`Tabs/TabsTrigger.vue:56-67`).
- Chuột phải, giữa, `Ctrl`+nhấn không focus (`:56-67`).
- Tự động: mọi focus chọn tab, kể cả focus do code (`:71-78`).
- `PageUp`/`PageDown` = `Home`/`End` (`RovingFocus/utils.ts:9-18`).
- `Home`/`End` bỏ qua tab disabled, khác Accordion (`RovingFocus/RovingFocusItem.vue:83-87`).
- Phím có modifier bị bỏ qua (`RovingFocus/RovingFocusItem.vue:80-81`).
- Panel luôn là tab stop, kể cả khi chứa phần tử focus được (`Tabs/TabsContent.vue:68`).
- Panel ẩn (có giữ) không tìm được, khác Accordion/Collapsible (`Tabs/TabsContent.vue:67`).
- Chế độ kích hoạt chỉ đọc lúc setup (`Tabs/TabsRoot.vue:104`). Quirk.
- Docs lệch source: bảng phím chỉ tả chế độ tự động và thiếu `PageUp`/`PageDown` (docs `tabs.md:199-230`).

### Pagination

`Pagination/` · dùng bởi: shadcn `pagination` (Root, List, ListItem, First, Prev, Next, Last, Ellipsis). Link trang là `<a>` riêng của shadcn (shadcn: `pagination/PaginationLink.vue:19`). Ellipsis có chữ ẩn "More pages" (shadcn: `pagination/PaginationEllipsis.vue:28`).

**Part:**
- Root: `nav`, sở hữu trang hiện tại.
- List: dãy ô trang và ellipsis.
- Page item: nút một trang.
- Ellipsis: ô lược trang.
- First, Prev, Next, Last: nút điều hướng.

**Model:**
- Sở hữu: trang hiện tại, đếm từ 1. Mặc định 1 (`Pagination/PaginationRoot.vue:56-62`).
- Caller cấu hình, mặc định (`:25`, `:34-39`, `:56-62`): tổng số mục (0); số mục mỗi trang (bắt buộc); số trang kề mỗi bên `s` (2); hiện trang đầu/cuối và ellipsis (không); disabled; kẹp trang (không).
- Số trang N = max(1, ⌈tổng / mỗi trang⌉), mỗi trang = 0 tính là 1 (`:87`).
- Người dùng ghi, luôn kẹp vào [1, N] (`:91-96`):
  - nút trang → trang đó (`Pagination/PaginationListItem.vue:34`);
  - First → 1; Prev → trang − 1; Next → trang + 1; Last → N (`Pagination/PaginationFirst.vue:27`, cùng dòng ở `PaginationPrev.vue`, `PaginationNext.vue`, `PaginationLast.vue`).
- Caller ghi: trang ngoài [1, N] được giữ nguyên, trừ khi bật kẹp. Bật kẹp: trang ngoài khoảng được sửa và báo caller đúng một lần (`Pagination/PaginationRoot.vue:91-106`; `Pagination/Pagination.test.ts:310-381`).
- N giảm, không kẹp: trang hiện tại giữ nguyên (`Pagination/Pagination.test.ts:265-307`).
- Mỗi lần trang đổi báo caller.

**Bàn phím:**

| Phím | Kết quả |
|---|---|
| `Tab`, `Shift+Tab` | Qua từng nút enabled. |
| `Enter`, `Space` | Kích hoạt nút đang focus (nút native). |
| Mũi tên | Không xử lý. |

**Pointer:**
- Click nút: như ở Model. Nút disabled không phản hồi (`Pagination/PaginationFirst.vue:27`; `Pagination/PaginationListItem.vue:34`).

**Trạng thái:**
- Dãy ô: suy ra từ trang p, N, s; không ai ghi (`Pagination/utils.ts:20-93`).
  - Không hiện đầu/cuối: không bao giờ có ellipsis. Cửa sổ cố định 2s+1 trang quanh p, dịch vào trong ở hai đầu. N < 2s+1 thì hiện hết (`:78-92`).
  - Hiện đầu/cuối, N ≤ 2s+5: hiện hết, không ellipsis (`:37-55`, `:75-76`).
  - Hiện đầu/cuối, N > 2s+5: luôn đúng 2s+5 ô. Ellipsis trái khi p − s > 3; phải khi p + s < N − 2 (`:41-55`). Mỗi ellipsis thay ít nhất 2 trang.
  - Dạng, với k = 2s+3: `1..k … N` | `1 … N−k+1..N` | `1 … p−s..p+s … N` (`:57-73`).
  - Ví dụ N = 10, s = 1: p = 1 → `1 2 3 4 5 … 10`; p = 5 → `1 … 4 5 6 … 10`; p = 7 → `1 … 6 7 8 9 10`. s = 2, p = 5 → `1 2 3 4 5 6 7 … 10` (`Pagination/Pagination.test.ts:173-194`).
- Nút cạnh disabled (`:18` ở mỗi file `PaginationFirst.vue`, `PaginationPrev.vue`, `PaginationNext.vue`, `PaginationLast.vue`):
  - First: p = 1. Prev: p ≤ 1. Next: p ≥ N. Last: p = N.
  - Cả bốn khi Root disabled.
- Tổng = 0: N = 1, cả bốn nút disabled (`Pagination/Pagination.test.ts:241-263`).
- Page item chỉ disabled khi Root disabled (`Pagination/PaginationListItem.vue:22`).

**ARIA và data:**
- Root: `nav`, không có tên. shadcn cũng không đặt tên (không có `aria-label` trong shadcn `pagination/`).
- List: không role (`Pagination/PaginationList.vue:38-41`).
- Page item: tên "Page N" (`Pagination/PaginationListItem.vue:29`); trang hiện tại có `aria-current=page` (`:30`); chữ mặc định là số trang (`:36`).
- First/Prev/Next/Last: tên "First Page", "Previous Page", "Next Page", "Last Page"; chữ mặc định "First page", "Prev page", "Next page", "Last page"; disabled bằng `disabled` native (`Pagination/PaginationFirst.vue:24-29`, cùng dòng ở ba file kia).
- Ellipsis: ký tự "…", không ARIA (`Pagination/PaginationEllipsis.vue:16-21`).
- Look cần đọc: ô nào là trang hiện tại; nút nào disabled; ô là trang hay ellipsis.

**RTL:**
- reka không xử lý. Icon Prev/Next của shadcn lật trong RTL, chỉ là look (shadcn: `pagination/PaginationPrevious.vue:36`, `pagination/PaginationNext.vue:37`).

**Cơ chế DOM không port:**
- Không đáng kể: Pagination chỉ là logic dãy trang và nút.

**Dễ sót:**
- Ellipsis chỉ có khi bật hiện đầu/cuối. Số ô cố định 2s+5, nên nút không đổi chỗ khi chuyển trang (`Pagination/utils.ts:37-76`).
- Next tới trang cuối: nút thành `disabled` native, focus rơi khỏi nút (`Pagination/PaginationNext.vue:18`, `:26`). Quirk: S6 có thể giữ focus.
- Caller đặt trang ngoài khoảng (không kẹp): không ô nào là trang hiện tại.
- Tên và chữ tiếng Anh cố định (`Pagination/PaginationListItem.vue:29`; `Pagination/PaginationFirst.vue:24`, `:29`).
- Docs lệch source: List "giúp AT dùng được" (docs `pagination.md:74`), source không có ARIA ở List. Docs nhắc điều hướng mũi tên (`:243`, `:250`), source không có.

### ScrollArea

`ScrollArea/` · dùng bởi: shadcn `scroll-area` (Root, Viewport, một Scrollbar dọc + Thumb, Corner; kiểu mặc định `hover`)

**Part:**
- Root: khung, cũng là vùng hover.
- Viewport: vùng cuộn, tab stop.
- Scrollbar: track của một trục.
- Thumb: phần kéo trong track.
- Corner: ô góc khi có cả hai thanh.
- Virtualizer: danh sách ảo trong viewport.

**Model:**
- Sở hữu: vị trí cuộn mỗi trục; hiện/ẩn của từng thanh.
- Người dùng ghi vị trí: wheel, touch, phím trên viewport, kéo thumb, nhấn track, wheel trên thanh.
- Caller ghi: cuộn về đầu; cuộn về góc trên-trái; truy cập viewport (`ScrollArea/ScrollAreaRoot.vue:70-89`).
- Caller cấu hình, mặc định (`:53-56`): kiểu hiện thanh (`hover`); độ trễ ẩn (600 ms); `dir`; trục nào có part thanh.
- Trục không có part thanh thì không cuộn (`ScrollArea/ScrollAreaViewport.vue:59-60`; docs `scroll-area.md:151`).

**Bàn phím:**

| Phím | Kết quả |
|---|---|
| `Tab` | Viewport là tab stop (`ScrollArea/ScrollAreaViewport.vue:63`). |
| Mũi tên, `PageUp`/`PageDown`, `Home`/`End`, `Space` | Cuộn native của trình duyệt; reka không định nghĩa (docs `scroll-area.md:316-318`). |

- S6 không có cuộn native, nên phải tự làm các phím này theo Chrome.

**Pointer:**
- Wheel trên thanh: cuộn trục của thanh đó bằng `deltaY`, kể cả thanh ngang. Chỉ chặn trang cuộn theo khi 0 < vị trí < max (`ScrollArea/ScrollAreaScrollbarVisible.vue:56-73`; `ScrollArea/utils.ts:122-127`).
- Nhấn thumb rồi kéo: điểm nắm giữ nguyên dưới pointer (`ScrollArea/ScrollAreaScrollbarVisible.vue:75-82`, `:101-111`; `ScrollArea/utils.ts:99-120`).
- Nhấn track, chỉ nút chính: nhảy ngay để tâm thumb nằm dưới pointer, không smooth, rồi kéo tiếp tới khi thả (`ScrollArea/ScrollAreaScrollbarImpl.vue:41-73`; `ScrollArea/utils.ts:105-108`).
- Đang kéo: pointer ra ngoài thanh vẫn kéo; không chọn text (`ScrollArea/ScrollAreaScrollbarImpl.vue:41-57`).

**Trạng thái:**
- Kiểu hiện thanh, cho từng thanh:
  - `always`: luôn hiện (`ScrollArea/ScrollAreaScrollbar.vue:114-121`).
  - `auto`: hiện khi nội dung tràn trục đó. Kiểm lúc mount và khi kích thước đổi, trễ 10 ms (`ScrollArea/ScrollAreaScrollbarAuto.vue:23-43`).
  - `hover`: hiện khi pointer trong Root và có tràn. Pointer rời → ẩn sau độ trễ (`ScrollArea/ScrollAreaScrollbarHover.vue:27-65`).
  - `scroll`: hiện khi đang cuộn theo trục của nó. Ngừng cuộn 100 ms → chờ độ trễ → ẩn (`ScrollArea/ScrollAreaScrollbarScroll.vue:45-80`). Hover không giữ thanh (Quirk, xem Dễ sót).
  - `glimpse` (`ScrollArea/ScrollAreaScrollbarGlimpse.vue:28-84`): pointer vào Root → hiện, rồi ẩn sau độ trễ dù pointer còn trong; pointer rời lúc đó → ẩn ngay. Cuộn → hiện; ngừng 100 ms → ẩn sau độ trễ. Pointer vào trong lúc cuộn hoặc lúc chờ ẩn → giữ hiện tới khi pointer rời, rồi ẩn sau độ trễ. Cần có tràn (`:130-138`).
- Thumb: chỉ có khi 0 < viewport / nội dung < 1 (`ScrollArea/ScrollAreaScrollbarVisible.vue:48-51`).
  - Độ dài = max(track × viewport / nội dung, 18 px), "như macOS" (`ScrollArea/utils.ts:17-24`).
  - Vị trí bị kẹp, không chạy theo overscroll (`:73-93`).
  - Bám cuộn quán tính: theo dõi mỗi frame, dừng sau 100 ms không đổi (`:33-71`).
- Vị trí thanh: ngang ở đáy, chừa bề rộng corner ở cuối; dọc ở mép phải, chừa chiều cao corner ở đáy (`ScrollArea/ScrollAreaScrollbarX.vue:31-36`; `ScrollArea/ScrollAreaScrollbarY.vue:32-38`).
- Corner: chỉ khi có cả hai thanh và kiểu khác `scroll` (`ScrollArea/ScrollAreaCorner.vue:18-23`). Kích thước = rộng thanh dọc × cao thanh ngang; chỉ hiện khi cả hai > 0 (`ScrollArea/ScrollAreaCornerImpl.vue:12-43`).
- Có thanh ngang: nội dung rộng tối thiểu bằng bề rộng tự nhiên, không bị ép theo viewport (`ScrollArea/ScrollAreaViewport.vue:75`).
- Virtualizer: overscan 12 hàng, ước lượng 28 px mỗi hàng (`ScrollArea/ScrollAreaVirtualizer.vue:53-59`). Hàng đang focus và hàng kề luôn được giữ (`:70-91`).

**ARIA và data:**
- Không có role hay `aria-*` nào trong `ScrollArea/` (grep). Thanh, thumb, corner là trang trí; AT dùng viewport.
- Viewport: `tabindex=0` (`ScrollArea/ScrollAreaViewport.vue:63`).
- Virtualizer: caller tự đặt `aria-setsize`/`aria-posinset` cho hàng (docs `scroll-area.md:309-314`).
- Look cần đọc: mỗi thanh hiện/ẩn (kể cả lúc đang ẩn dần); hướng thanh; có thumb hay không; độ dài và vị trí thumb; kích thước corner; viewport đang focus.

**RTL:**
- Thanh dọc ở mép trái. Chỗ chừa corner của thanh ngang ở bên trái. Corner ở góc dưới-trái (`ScrollArea/ScrollAreaScrollbarY.vue:32-38`; `ScrollArea/ScrollAreaScrollbarX.vue:31-36`; `ScrollArea/ScrollAreaCornerImpl.vue:12-43`).
- Cuộn ngang bắt đầu từ mép phải; thumb ở bên phải khi ở đầu. Kéo thumb và nhấn track tính theo RTL (`ScrollArea/utils.ts:73-93`; `ScrollArea/ScrollAreaScrollbarVisible.vue:101-111`).
- Virtualizer cuộn tới hàng theo RTL (`ScrollArea/ScrollAreaVirtualizer.vue:61-69`).

**Cơ chế DOM không port:**
- Ẩn thanh native bằng style chèn, bật momentum touch (`ScrollArea/ScrollAreaViewport.vue:83-98`) → thanh tự vẽ đè lên vật lý cuộn native.
- `overflow: hidden` ở trục không có thanh (`:59-60`) → trục đó không cuộn.
- `min-width: fit-content` (`:75`) → nội dung ngang không co theo viewport.
- `ResizeObserver` + trễ 10 ms (`ScrollArea/ScrollAreaScrollbarAuto.vue:23-43`; `ScrollArea/ScrollAreaScrollbarImpl.vue:92-122`) → thanh và thumb theo kích thước nội dung.
- Vòng frame tới khi idle 100 ms (`ScrollArea/utils.ts:33-71`) → thumb bám cuộn quán tính.
- Listener wheel non-passive trên document (`ScrollArea/ScrollAreaScrollbarImpl.vue:75-90`) → wheel trên thanh cuộn trục của thanh.
- Pointer capture, tắt chọn text, tạm tắt cuộn smooth (`ScrollArea/ScrollAreaScrollbarImpl.vue:41-57`, `:63-73`) → kéo ra ngoài vẫn được, nhảy tức thời.
- Thanh `position: absolute`, biến CSS kích thước corner/thumb (`ScrollArea/ScrollAreaScrollbarImpl.vue:128`; `ScrollArea/ScrollAreaRoot.vue:131-146`) → thanh đè lên nội dung; look đọc kích thước.

**Dễ sót:**
- Trục không có part thanh không cuộn được, kể cả bằng wheel và touch (`ScrollArea/ScrollAreaViewport.vue:59-60`).
- Nhấn track: tâm thumb nhảy tới pointer rồi kéo tiếp. Không cuộn từng trang như thanh hệ thống (`ScrollArea/utils.ts:105-108`).
- Wheel trên thanh ngang cuộn ngang bằng `deltaY` (`ScrollArea/ScrollAreaScrollbarVisible.vue:56-73`).
- Thumb tối thiểu 18 px (`ScrollArea/utils.ts:17-24`).
- `hover`: cuộn bằng phím khi pointer ở ngoài thì thanh không hiện (`ScrollArea/ScrollAreaScrollbarHover.vue:27-53`).
- Quirk `scroll`: trạng thái "đang tương tác" không bao giờ tới vì không có sự kiện pointer vào/rời. Thanh ẩn dù pointer đang trên nó (`ScrollArea/ScrollAreaScrollbarScroll.vue:24-41`). S6 nên giữ thanh khi hover.
- Quirk: gỡ một thanh làm tắt cuộn cả hai trục (`ScrollArea/ScrollAreaScrollbar.vue:66-69`).
- Docs lệch source: `hover` "hiện khi cuộn hoặc hover" (`ScrollArea/ScrollAreaRoot.vue:38`; docs `meta/ScrollAreaRoot.md:33`, `:64`). Source chỉ hiện khi pointer vào.

### Splitter

`Splitter/` · dùng bởi: shadcn `resizable` (Group, Panel, ResizeHandle)

**Part:**
- Group: dãy panel theo một hướng; sở hữu layout.
- Panel: một vùng có ràng buộc kích thước.
- ResizeHandle: thanh giữa hai panel; kéo hoặc dùng phím.

**Model:**
- Sở hữu: layout = kích thước mỗi panel theo % của group, tổng luôn 100 (`Splitter/utils/layout.ts:266-271`; `Splitter/utils/validation.ts:33-94`).
- Ràng buộc mỗi panel, do caller đặt: min (0), max (100), gập được (không), kích thước khi gập (0), kích thước mặc định, đơn vị % hoặc px (`Splitter/utils/units.ts:68-75`).
- Thứ tự panel: thứ tự caller đặt; panel không đặt đứng trước (`Splitter/SplitterGroup.vue:318-336`).
- Khởi tạo: layout đã lưu nếu có. Không thì panel có kích thước mặc định giữ nó, phần còn lại chia đều. Rồi chuẩn hóa (`Splitter/utils/calculate.ts:138-183`; `Splitter/SplitterGroup.vue:340-400`).
- Chuẩn hóa: đưa tổng về 100, kẹp từng panel, phần dư cho panel đầu tiên nhận được (`Splitter/utils/validation.ts:33-94`).
- Người dùng ghi: kéo handle; phím trên handle; `Enter` gập/mở panel trước handle.
- Caller ghi, qua lệnh trên panel (`Splitter/SplitterGroup.vue:555-874`):
  - đặt kích thước: px đổi sang % (`:555-609`);
  - gập: chỉ panel gập được; nhớ kích thước trước khi gập (`:693-754`);
  - mở: về kích thước đã nhớ nếu ≥ min, không thì min (`:756-820`);
  - hỏi: kích thước; đang gập = kích thước ≤ kích thước gập, so gần đúng (`:844-874`).
- Đổi kích thước mặc định sau mount: bỏ qua (`Splitter/SplitterPanel.vue:120-132`).
- Báo caller:
  - layout mỗi lần đổi, theo đơn vị riêng của từng panel (`Splitter/SplitterGroup.vue:203-228`);
  - panel: gập, mở, đổi kích thước (mới, cũ) (`Splitter/SplitterPanel.vue:25-32`); lần báo đầu cũng báo gập hoặc mở (`Splitter/utils/callPanelCallbacks.ts:25-50`);
  - handle: bắt đầu và kết thúc kéo (`Splitter/SplitterResizeHandle.vue:96-129`).
- Lưu, khi caller đặt id lưu: ghi sau 100 ms không đổi (`Splitter/SplitterGroup.vue:31`, `:267-302`). Khóa theo id panel của caller, không có thì theo thứ tự + ràng buộc (`Splitter/utils/storage.ts:53-68`).
- Group đổi kích thước: panel px giữ px, panel % co giãn (`Splitter/utils/units.ts:90-173`; `Splitter/SplitterGroup.vue:402-458`). Lần đo đầu < 50 px rồi lớn hơn 10 lần → khởi tạo lại (`:420-424`).
- Caller cấu hình: hướng (bắt buộc); bước phím (10, đơn vị % group) (`Splitter/SplitterGroup.vue:101-105`); handle disabled.
- Ý nghĩa một lần dời handle một lượng d (% group; d > 0 = về phía cuối):
  - Kéo: d tính từ layout lúc bắt đầu kéo, không cộng dồn (`Splitter/SplitterGroup.vue:470`, `:500`). d = 0 bỏ qua (`:486-487`).
  - |d| bị giới hạn bởi tổng phần còn lớn được của các panel phía lớn lên (`Splitter/utils/layout.ts:122-159`).
  - Phía co: panel kề handle co trước; thiếu thì lan sang panel xa hơn cùng phía (`:161-200`).
  - Phía lớn: panel kề handle nhận; chạm giới hạn thì phần dư lan ra xa hơn (`:212-261`).
  - Không panel nào đổi, hoặc tổng khác 100 → giữ layout cũ (`:205-210`, `:266-271`).
  - Panel gập được, nhỏ hơn min: dưới điểm giữa (kích thước gập, min) → gập; còn lại → min. Panel khác kẹp vào min. Sau đó kẹp max, làm tròn 10 chữ số (`Splitter/utils/resizePanel.ts:21-36`). Áp ngay trong lúc kéo.
  - Phím: panel đang gập mở thẳng tới min trong một lần nhấn; panel đang ở min gập trong một lần nhấn (`Splitter/utils/layout.ts:63-118`).

**Bàn phím:** (focus trên handle)

| Phím | Kết quả |
|---|---|
| `Tab` | Mỗi handle là tab stop, kể cả handle disabled (`Splitter/SplitterResizeHandle.vue:40-42`). |
| `ArrowRight` (ngang), `ArrowDown` (dọc) | d = +bước (mặc định 10) (`Splitter/utils/calculate.ts:47-78`). |
| `ArrowLeft` (ngang), `ArrowUp` (dọc) | d = −bước. |
| `Shift` + mũi tên trên | d = ±100, tới giới hạn (`:51-52`). |
| `End` | d = +100: panel trước handle lớn nhất có thể (`:70-71`). |
| `Home` | d = −100: panel trước handle nhỏ nhất có thể (`:73-74`). |
| Mũi tên khác trục | Không đổi (`:56-78`; `Splitter/SplitterGroup.vue:486-487`). |
| `Enter` | Panel trước handle, nếu gập được: đang gập → min; không → gập (`Splitter/utils/composables/useWindowSplitterPanelGroupBehavior.ts:136-172`). |
| `F6`, `Shift+F6` | Focus handle kế / trước trong group, bỏ qua handle disabled khác, vòng (`Splitter/utils/composables/useWindowSplitterBehavior.ts:48-76`). |
| Mọi phím đổi kích thước trên handle disabled | Bỏ qua, kể cả `Enter` (`useWindowSplitterBehavior.ts:39-41`; `useWindowSplitterPanelGroupBehavior.ts:131-133`). |

**Pointer:**
- Vùng bắt: hình chữ nhật handle nới thêm 15 px (pointer thô) hoặc 5 px (pointer mịn) (`Splitter/SplitterResizeHandle.vue:136-140`; `Splitter/utils/registry.ts:184-265`).
- Phần tử khác nằm đè lên handle thì chặn vùng bắt (`Splitter/utils/registry.ts:184-265`).
- Pointer trong vùng bắt → handle ở trạng thái hover (`Splitter/SplitterResizeHandle.vue:96-129`).
- Nhấn bất kỳ nút chuột nào → bắt đầu kéo (`Splitter/utils/registry.ts:103-117`). Quirk.
- Vùng bắt của hai handle khác hướng chồng nhau → kéo cả hai cùng lúc, theo 2 chiều (`:300-301`).
- Tập handle được kéo cố định từ lúc nhấn tới lúc thả (`:132-136`).
- Đang kéo: nội dung panel không nhận pointer (`Splitter/utils/style.ts:132-134`); không chọn text; touch không cuộn trang (`Splitter/SplitterResizeHandle.vue:166-169`).
- Mở context menu → kết thúc kéo (`Splitter/utils/registry.ts:334`).
- Con trỏ (`Splitter/utils/style.ts:16-58`, `:69-89`):
  - `ew-resize` (ngang), `ns-resize` (dọc), `move` ở giao điểm.
  - Chạm min/max → con trỏ một chiều (`e`/`w`/`n`/`s-resize` và góc) chỉ hướng còn kéo được.
  - Giữ trên toàn trang suốt lúc kéo.
  - Chỉ cho chuột và touch, không cho phím (`Splitter/SplitterGroup.vue:508-537`).

**Trạng thái:**
- Handle: nghỉ / hover / đang kéo (`Splitter/SplitterResizeHandle.vue:96-129`); active do pointer hay do phím (`:179`); disabled.
- Panel: kích thước; gập / mở, chỉ với panel gập được (`Splitter/SplitterPanel.vue:189`).
- Group chỉ có một panel: panel luôn đầy (`Splitter/utils/style.ts:116-118`).

**ARIA và data:**
- Handle: `role=separator` (`Splitter/SplitterResizeHandle.vue:172`).
- Handle: `aria-controls` → panel trước handle; `aria-valuenow`, `aria-valuemin`, `aria-valuemax` = % làm tròn số nguyên (`Splitter/utils/composables/useWindowSplitterPanelGroupBehavior.ts:52-81`).
- `aria-valuemax` = min(max, 100 − Σ min các panel khác). `aria-valuemin` = max(min, 100 − Σ max các panel khác) (`Splitter/utils/calculate.ts:94-136`).
- Không có `aria-orientation`, `aria-label`, `aria-disabled` ở đâu trong `Splitter/` (grep). S6 nên có.
- Look cần đọc: handle nghỉ/hover/kéo; active do pointer hay phím; disabled; hướng group; panel gập/mở và kích thước; hình con trỏ.

**RTL:**
- Hướng chỉ lấy từ cấu hình chung; group không có tùy chọn riêng (`Splitter/SplitterGroup.vue:121`).
- Ngang RTL: đảo dấu d cho cả kéo và phím, kể cả `Home`/`End` (`Splitter/SplitterGroup.vue:489-492`). Quirk: trong RTL `Home` làm panel trước lớn nhất.

**Cơ chế DOM không port:**
- `localStorage`, khóa tiền tố `reka:` (`Splitter/utils/storage.ts:45-47`) → layout bền qua phiên.
- Listener trên toàn trang + lề bắt (`Splitter/utils/registry.ts:103-117`, `:184-265`) → vùng kéo lớn hơn handle.
- So stacking order (`registry.ts:184-265`) → phần tử nằm đè chặn handle.
- Media query pointer thô, đọc một lần lúc tải (`registry.ts:34-39`) → lề 15 px cho touch.
- Style con trỏ toàn trang (`Splitter/utils/style.ts:69-89`) → con trỏ không đổi khi pointer rời handle lúc kéo.
- `pointer-events: none` trong panel khi kéo (`style.ts:132-134`) → nội dung panel không hover, không nhận click lúc kéo.
- `user-select: none`, `touch-action: none` (`Splitter/SplitterResizeHandle.vue:166-169`) → không chọn text, không cuộn khi kéo.
- `ResizeObserver` trên group (`Splitter/SplitterGroup.vue:241-265`) → panel px giữ px.
- Style flex theo % (`:955-973`) → layout.

**Dễ sót:**
- `Enter` mở panel gập về min, không về kích thước trước khi gập. Lệnh mở của caller lại về kích thước đã nhớ (`useWindowSplitterPanelGroupBehavior.ts:155-158` vs `Splitter/SplitterGroup.vue:771-790`). Docs nói `Enter` về vị trí trước (docs `splitter.md:276`). S6 chọn một và ghi lại.
- Gập bắt ở điểm giữa (kích thước gập, min), áp ngay khi kéo (`Splitter/utils/resizePanel.ts:21-33`).
- Kéo tính từ layout lúc bắt đầu: kéo quá giới hạn rồi quay lại, handle không trôi khỏi pointer (`Splitter/SplitterGroup.vue:470`, `:500`).
- `Shift`+mũi tên = 100 (`Splitter/utils/calculate.ts:51-52`); `F6` vòng qua handle. Docs không ghi cả hai.
- Handle disabled vẫn focus được; `F6` bỏ qua nó (`Splitter/SplitterResizeHandle.vue:40-42`; `useWindowSplitterBehavior.ts:48-76`; `Splitter/SplitterResizeHandle.test.ts:403-443`).
- Bước phím là % group, kể cả với panel px. Quirk.
- Lần báo đầu cũng báo gập hoặc mở (`Splitter/utils/callPanelCallbacks.ts:25-50`).
- Docs lệch source:
  - min mặc định 10 (`Splitter/SplitterPanel.vue:17`; docs `meta/SplitterPanel.md:151`), thực tế 0 (`Splitter/utils/units.ts:68-75`).
  - Trạng thái gập ghi ở Group (docs `splitter.md:61-72`), thực tế ở Panel.
  - "Hỗ trợ RTL" (docs `splitter.md:23`): chỉ qua cấu hình chung.

### Avatar

`Avatar/` · dùng bởi: shadcn `avatar` (Root, Image, Fallback; Badge, Group, GroupCount là của shadcn)

**Part:**
- Root: khung (`span`) (`Avatar/AvatarRoot.vue:21-29`).
- Image: ảnh.
- Fallback: nội dung thay thế (chữ cái, icon).

**Model:**
- Sở hữu: trạng thái tải ảnh: chưa bắt đầu → đang tải → đã tải | lỗi (`Avatar/AvatarRoot.vue:21-29`).
- Không ai ghi trực tiếp. Trạng thái suy từ nguồn ảnh (`Avatar/utils.ts:7-18`, `:40-66`):
  - không có nguồn → lỗi;
  - ảnh đã có sẵn (tải xong, rộng > 0) → đã tải ngay, không qua đang tải;
  - còn lại → đang tải, rồi đã tải hoặc lỗi;
  - đổi nguồn → tính lại.
- Root không quay về "chưa bắt đầu" (`Avatar/AvatarImage.vue:36-44`).
- Mỗi lần trạng thái ảnh đổi báo caller, từ lần đầu (`:36-44`).
- Caller cấu hình: độ trễ trước khi hiện fallback (ms); mặc định không trễ (`Avatar/AvatarFallback.vue:24`).

**Bàn phím:**
- Không có.

**Pointer:**
- Không có.

**Trạng thái:**
- Ảnh chỉ hiện khi đã tải; trước đó ẩn (`Avatar/AvatarImage.vue:48-58`).
- Fallback hiện khi chưa tải xong (`Avatar/AvatarFallback.vue:41`) và đã qua độ trễ. Độ trễ tính từ lúc fallback xuất hiện (`:26-36`).
- Không bao giờ thấy ảnh vỡ: ảnh được tải ngoài cây (`Avatar/utils.ts:23-31`).

**ARIA và data:**
- Image: `role=img` (`Avatar/AvatarImage.vue:48-58`).
- Root, Fallback: không role.
- Look cần đọc: trạng thái tải.

**RTL:**
- Không ảnh hưởng.

**Cơ chế DOM không port:**
- Đối tượng `Image` ngoài cây để tải trước (`Avatar/utils.ts:23-31`) → biết trạng thái mà không hiện ảnh vỡ.
- Ảnh ẩn bằng style tới khi tải xong (`Avatar/AvatarImage.vue:48-58`) → không nháy ảnh dở.

**Dễ sót:**
- Ảnh đã cache → đã tải ngay, fallback không nháy (`Avatar/utils.ts:7-18`).
- Độ trễ 0 thì fallback không bao giờ hiện: hẹn giờ chỉ chạy khi độ trễ khác 0, còn "hiện ngay" chỉ khi không đặt độ trễ (`Avatar/AvatarFallback.vue:24-36`). Quirk: S6 coi 0 là hiện ngay.
- Độ trễ tính từ lúc fallback xuất hiện, không từ lúc bắt đầu tải (`:26-36`).
- Docs lệch source: ảnh "chỉ render khi đã tải" (docs `avatar.md:59`). Source render ngay và ẩn đi.

### AspectRatio

`AspectRatio/` · dùng bởi: shadcn `aspect-ratio`

**Part:**
- Một khung, một con.

**Model:**
- Không có trạng thái người dùng.
- Caller cấu hình: tỉ lệ rộng / cao, mặc định 1 (`AspectRatio/AspectRatio.vue:22-24`).

**Bàn phím:**
- Không có.

**Pointer:**
- Không có.

**Trạng thái:**
- Khung rộng 100% vùng chứa; cao = rộng / tỉ lệ (`AspectRatio/AspectRatio.vue:40-41`).
- Con lấp đầy khung (`:44-49`).
- Kích thước khung không phụ thuộc nội dung. Nội dung lớn hơn thì tràn ra, không bị cắt (`:40-49`).

**ARIA và data:**
- Không ARIA.
- Look cần đọc: kích thước khung.

**RTL:**
- Không ảnh hưởng.

**Cơ chế DOM không port:**
- `padding-bottom` theo % + con `position: absolute; inset: 0` (`AspectRatio/AspectRatio.vue:40-49`) → cao theo rộng, con không ảnh hưởng kích thước.

**Dễ sót:**
- Nội dung không đẩy khung lớn ra, cũng không bị cắt (`AspectRatio/AspectRatio.vue:40-49`). Taffy có tỉ lệ trong style (`references/taffy/src/style/mod.rs:764`); nếu thay mẹo padding bằng nó, cần kiểm lại bảo đảm này.

### Carousel

`Carousel/` · dùng bởi: không component shadcn-vue nào. shadcn `carousel` dùng embla-carousel-vue (shadcn: `carousel/useCarousel.ts:3`, `:12`). Ghi ngắn.

**Part:**
- Root (region), Viewport (vùng cuộn, tab stop), Content, slide, Prev, Next, nhóm indicator, indicator, nút xoay.

**Model:**
- Sở hữu: snap hiện tại (chỉ số từ 0); đang xoay / dừng.
- Người dùng ghi snap: kéo, wheel, phím trên viewport, Prev/Next, indicator. Ghi xoay: nút xoay; tương tác của người dùng cũng dừng xoay (`Carousel/useCarousel.ts:37-46`).
- Caller ghi snap: ngoài khoảng bị từ chối (`:369-379`). Số snap giảm → kẹp về snap cuối (`:383-386`).
- Bước được = có hơn một snap và (có vòng hoặc chưa tới đầu/cuối) (`:446-447`).
- Caller cấu hình, mặc định (`Carousel/CarouselRoot.vue:188-200`): ngang; căn đầu; 1 slide mỗi bước; bỏ snap thừa ở cuối; kéo được; không tự xoay.
- Tự xoay: 4000 ms một bước, dừng khi người dùng tương tác, không dừng ở snap cuối (`Carousel/useCarousel.ts:37-46`). Tự cuộn: 60 px/s (`:48-57`).

**Bàn phím:**

| Phím | Kết quả |
|---|---|
| `Tab` | Viewport là tab stop (`Carousel/useCarouselViewport.ts:1039`). |
| Viewport: `ArrowRight` (ngang LTR), `ArrowLeft` (ngang RTL), `ArrowDown` (dọc) | Snap kế (`:821`, `:825-826`). |
| Viewport: phím ngược lại | Snap trước (`:822`, `:827-828`). |
| Viewport: `Home`, `End` | Snap đầu, snap cuối (`:829-832`). |
| Viewport, khi focus ở phần tử con, hoặc kèm `Alt`/`Ctrl`/`Meta` | Bỏ qua (`:819-820`). |
| Nhóm indicator: `ArrowRight`, `ArrowDown` | Indicator kế, focus và chọn luôn, vòng; RTL đảo trục ngang (`Carousel/CarouselIndicatorGroup.vue:44-60`). |
| Nhóm indicator: `ArrowLeft`, `ArrowUp` | Indicator trước, focus và chọn, vòng. |
| Nhóm indicator: `Home`, `End` | Indicator đầu, cuối (`:52-55`). |
| Focus bằng bàn phím vào carousel | Dừng xoay (`Carousel/useCarouselRotation.ts:124-132`). |

- Phím đã xử lý trên viewport tính là tương tác và ngắt cuộn đang chạy (`Carousel/useCarouselViewport.ts:835-839`).

**Pointer:**
- Touch: cuộn native; xoay tạm dừng khi ngón tay chạm (`Carousel/useCarouselViewport.ts:522-560`; `Carousel/useCarouselRotation.ts:27`).
- Chuột/bút: kéo giả lập. Bắt đầu sau 3 px; quá 10 px thì nuốt click; không bắt đầu từ trường nhập (`Carousel/useCarouselViewport.ts:20`, `:22`, `:522-560`).
- Cử chỉ chủ yếu theo trục kia → để trang cuộn (`:621-679`).
- Thả: tối đa một snap mỗi cử chỉ, trừ khi bật bỏ qua snap (`:681-726`, `:715-722`). Vận tốc lấy trong 80 ms cuối, chiếu 120 ms (`:27`, `:29`). Quán tính giảm 0.996 mỗi ms (`:31`, `:728-750`).
- Fade: đổi slide khi vuốt 40 px hoặc 0.3 px/ms (`:24-25`).
- Wheel: một trang mỗi 30 đơn vị delta; nghỉ 180 ms là cử chỉ mới (`:33-34`, `:772-816`).
- Kéo quá đầu/cuối: kháng 0.3, bật về trong 250 ms (`:38`, `:600-618`).
- Chuột hover (chỉ chuột) tạm dừng xoay (`Carousel/useCarouselRotation.ts:116-123`).

**Trạng thái:**
- Snap hiện tại; slide trong tầm nhìn; đang kéo; đang xoay / dừng; ở đầu / cuối.
- Báo caller "đã dừng" đúng một lần mỗi lần đổi snap (`Carousel/useCarouselViewport.ts:199-206`).
- Sau mọi cuộn: về snap gần nhất; hai snap gần như nhau (trong 0.5 px) thì giữ snap hiện tại (`:428-452`; `Carousel/utils.ts:170-184`).
- Xoay tạm dừng khi: chuột hover, trang ẩn, carousel ngoài màn hình, ngón tay đang chạm (`Carousel/useCarouselRotation.ts:27`).
- Hệ thống giảm chuyển động: xoay dừng ngay lúc mount (`:29-37`); cuộn do code đi tức thời, không thì smooth (`Carousel/useCarouselViewport.ts:377`).
- Tới snap cuối khi xoay: dừng nếu caller chọn, không thì về snap 0 (`Carousel/useCarouselRotation.ts:80-102`). Đổi snap → đếm giờ lại từ đầu (`:103-112`).
- Slide đang có focus ra khỏi tầm nhìn → focus được chuyển đi, không mất (`Carousel/useCarouselViewport.ts:208-238`).

**ARIA và data:**
- Root: `role=region`, `aria-roledescription="carousel"` (`Carousel/CarouselRoot.vue:249-250`).
- Prev/Next: tên "Previous slide"/"Next slide"; `aria-controls` → content; ở đầu/cuối: `aria-disabled`, vẫn focus được (`Carousel/useCarousel.ts:74-95`, `:80-82`).
- Nhóm indicator: `role=tablist`, tên "Choose slide to display" (`Carousel/CarouselIndicatorGroup.vue:69-70`).
- Indicator trong nhóm: `role=tab`, `aria-selected`, chỉ tab hiện tại là tab stop, tên "Slide N", `aria-controls` → các slide của snap đó (`Carousel/useCarousel.ts:114-123`).
- Indicator đứng riêng: tên "Go to slide N", `aria-controls` → content, cái hiện tại `aria-disabled` (`:114`, `:119`, `:124`). Nhấn indicator hiện tại: không làm gì (`:126-127`).
- Nút xoay: tên "Stop slide rotation" / "Start slide rotation", không có trạng thái pressed (`:136-151`; `Carousel/CarouselAutoplayToggle.vue:25`).
- Content: `aria-live=polite`, thành `off` khi đang xoay (`Carousel/useCarousel.ts:162-163`).
- Slide: `role=group` + `aria-roledescription="slide"`, hoặc `tabpanel` khi có nhóm indicator; tên "N of M"; ngoài tầm nhìn: `aria-hidden` và không tương tác được (`:205-210`, `:244-255`).
- Look cần đọc: slide hiện tại; trong tầm nhìn; đang kéo; đang xoay/dừng; ở đầu/cuối; tiến độ tới lượt xoay kế (0..1); chiều cao theo slide; khoảng xoay.

**RTL:**
- Phím viewport và indicator đảo trục ngang (`Carousel/useCarouselViewport.ts:821-822`; `Carousel/CarouselIndicatorGroup.vue:44-46`).

**Cơ chế DOM không port:**
- CSS scroll snap, bắt buộc trừ khi kéo tự do, fade, hoặc tạm ngưng (`Carousel/useCarousel.ts:185-198`; `Carousel/useCarouselViewport.ts:152-156`) → dừng đúng snap.
- `scrollend` + 120 ms dự phòng (`Carousel/useCarouselViewport.ts:44`) → biết lúc cuộn xong.
- Thử lại 2 lần cho cuộn do code (`:36`) → cuộn do code tới đúng snap.
- `inert` + `aria-hidden` (`Carousel/useCarousel.ts:254-255`) → slide ngoài tầm nhìn không focus, AT không đọc.
- Theo dõi trang ẩn và carousel ngoài màn hình (`Carousel/useCarouselRotation.ts:27`) → không xoay khi không ai thấy.
- Media query giảm chuyển động (`:29-37`; `Carousel/useCarouselViewport.ts:377`) → theo cài đặt hệ thống.

**Dễ sót:**
- Prev/Next ở đầu/cuối dùng `aria-disabled`, giữ focus (`Carousel/useCarousel.ts:80-82`); khác Pagination.
- Live region tắt khi đang xoay (`:162-163`).
- Phím viewport chỉ chạy khi chính viewport có focus (`Carousel/useCarouselViewport.ts:819`).
- Mũi tên trong nhóm indicator vừa dời focus vừa chọn (`Carousel/CarouselIndicatorGroup.vue:58-60`).
- Docs lệch source: nhóm indicator chỉ ghi `ArrowRight`/`ArrowLeft` (docs `carousel.md:501-502`); source có thêm `ArrowUp`/`ArrowDown`, `Home`/`End`.
- Verdict của `carousel` so với embla, không với reka.

### Ghi chú chung cho nhóm 8
- Mô hình tab stop khác nhau:

| Primitive | Tab stop | Mũi tên |
|---|---|---|
| Accordion | mỗi trigger | dời focus giữa trigger, vòng |
| Tabs | một tab + panel | dời focus; chế độ tự động thì chọn luôn |
| Pagination | mỗi nút | không |
| ScrollArea | viewport | cuộn |
| Splitter | mỗi handle, cả handle disabled | đổi kích thước; `F6` qua handle |
| Carousel | viewport, nút, một indicator | viewport: đổi snap; indicator: focus và chọn |

- Disabled có hai kiểu:
  - `disabled` native, mất focus: Accordion, Collapsible, Tabs, Pagination.
  - `aria-disabled`, giữ focus: Carousel (Prev/Next, indicator đứng riêng).
  - Splitter: handle disabled giữ focus nhưng không báo `aria-disabled`.
  - S6 chọn một quy tắc cho cả nhóm và ghi lại.
- Nội dung đóng có ba mức:
  - rời cây: mặc định ở Accordion, Collapsible, Tabs;
  - ẩn, find-in-page tìm thấy và mở: Accordion, Collapsible khi giữ nội dung;
  - ẩn hẳn: Tabs khi giữ panel.
- Animation: nội dung đang đóng còn hiện tới hết animation; nội dung mở sẵn lúc mount không animate (Accordion, Collapsible, Tabs). Thanh ScrollArea cũng ẩn qua animation, trừ `always`.
- RTL chỉ đảo mũi tên ngang. Splitter không có `dir` riêng và đảo cả `Home`/`End` (quirk). ScrollArea đổi mép thanh dọc và corner.
- Tùy chọn chỉ đọc lúc setup: Accordion (cho đóng hết, hướng), Tabs (chế độ kích hoạt). S6 nên phản ứng khi chúng đổi.
- Chuỗi tiếng Anh cố định cần bản địa hóa: Pagination ("Page N", "First Page", …), Carousel ("Previous slide", "N of M", "Choose slide to display", …), shadcn ("More pages").
- Giá trị hành vi:

| Giá trị | Nguồn |
|---|---|
| Thanh cuộn ẩn sau 600 ms | `ScrollArea/ScrollAreaRoot.vue:53-56` |
| Ngừng cuộn sau 100 ms (thanh `scroll`, `glimpse`; thumb) | `ScrollArea/ScrollAreaScrollbarScroll.vue:55`; `ScrollArea/ScrollAreaScrollbarGlimpse.vue:64`; `ScrollArea/utils.ts:33-71` |
| Kiểm tràn trễ 10 ms | `ScrollArea/ScrollAreaScrollbarAuto.vue:23-43` |
| Thumb tối thiểu 18 px | `ScrollArea/utils.ts:17-24` |
| Virtualizer: overscan 12, ước lượng 28 px | `ScrollArea/ScrollAreaVirtualizer.vue:53-59` |
| Lề bắt handle 15 px (thô) / 5 px (mịn) | `Splitter/SplitterResizeHandle.vue:136-140` |
| Bước phím 10 %, `Shift` 100 % | `Splitter/utils/calculate.ts:47-55` |
| Lưu layout sau 100 ms | `Splitter/SplitterGroup.vue:31` |
| Khởi tạo lại: lần đo đầu < 50 px rồi > 10 lần | `Splitter/SplitterGroup.vue:420-424` |
| Kích thước panel làm tròn 10 chữ số | `Splitter/utils/resizePanel.ts:21-36` |
| Pagination: s = 2; 2s+5 ô khi hiện đầu/cuối, 2s+1 khi không | `Pagination/PaginationRoot.vue:56-62`; `Pagination/utils.ts:37`, `:79` |
| Carousel xoay 4000 ms, tự cuộn 60 px/s | `Carousel/useCarousel.ts:37-57` |
| Carousel kéo: 3 px bắt đầu, 10 px nuốt click | `Carousel/useCarouselViewport.ts:20-22` |
| Carousel fade: 40 px hoặc 0.3 px/ms | `Carousel/useCarouselViewport.ts:24-25` |
| Carousel thả: chiếu 120 ms, vận tốc trong 80 ms, quán tính 0.996/ms | `Carousel/useCarouselViewport.ts:27-31` |
| Carousel wheel: 30 delta mỗi trang, 180 ms giữa cử chỉ | `Carousel/useCarouselViewport.ts:33-34` |
| Carousel overscroll: kháng 0.3, bật về 250 ms | `Carousel/useCarouselViewport.ts:38`, `:600-618` |
| Carousel cuộn do code: 2 lần thử lại, `scrollend` dự phòng 120 ms | `Carousel/useCarouselViewport.ts:36`, `:44` |
| Carousel hai snap ngang nhau: trong 0.5 px | `Carousel/utils.ts:170-184` |

- Docs hay thiếu phím: `PageUp`/`PageDown` (Tabs); `Shift`+mũi tên và `F6` (Splitter); `ArrowUp`/`ArrowDown`/`Home`/`End` (nhóm indicator Carousel). Lấy source làm chuẩn.
- Khối dùng chung được cấu hình:
  - `RovingFocus`: Tabs (hướng, `dir`, vòng; `Shift`+mũi tên bị bỏ qua).
  - `Presence`: Collapsible, Accordion (qua Collapsible), panel Tabs, thanh ScrollArea (trừ `always`).
  - `Collection` + điều hướng mũi tên, không qua `RovingFocus`: Accordion.
  - controlled/uncontrolled: Accordion, Collapsible, Tabs, Pagination.
- shadcn `carousel` dùng embla: verdict so với embla, không với reka.

## 13. Nguồn

- reka-ui 2.11.0, commit `f67a0e0`, trong `references/reka-ui/`:
  - source và test: `packages/core/src/`;
  - docs: `docs/content/docs/` và `docs/content/meta/`. Khi docs nói khác source, source thắng.
- shadcn-vue `d1bf1b3`, pin của verdict, trong `crates/shadcn/verdict/.shadcn-vue/apps/v4/registry/`:
  - component: `bases/reka/ui/`; ví dụ: `bases/reka/examples/`; style: `styles/`;
  - chỉ đọc: `crates/shadcn/verdict/fetch.sh` sở hữu bản checkout này.
- Vue: `references/vuejs/packages/`.
- vueuse 15.0.0, từ một bản cài cục bộ, không có trong `references/`.
- `gui` hôm nay, trong `crates/gui/src/`:
  - `focus.rs`;
  - `element.rs` (`deferred`, `Placement`, `on_click_outside`) và `placement.rs`;
  - `members.rs`;
  - `component.rs` (`Bind`, `use_model`, `after_layout`, `provide`, `use_context`);
  - `clock.rs`, `motion.rs`, `access.rs`, `time_input.rs`.
- Quyết định và kế hoạch:
  - [ADR-0022](adr/0022-a-style-is-a-typed-value-components-read.md): style là giá trị có kiểu; ba crate
    `gui` ← `primitives` ← `shadcn`.
  - ADR-0007 (motion), ADR-0009 (IME và AccessKit), ADR-0013 (component và scheduler), ADR-0014 (layout và danh sách
    ảo), ADR-0015 (ba thread, event có dấu), ADR-0016 (property tree), ADR-0021 (AccessKit chỉ gửi phần đổi). Danh
    sách ở [`docs/adr/README.md`](adr/README.md).
  - M10 trong [`docs/plan-2026-11-21.md`](plan-2026-11-21.md); nghiên cứu style ở
    [`docs/research-2026-11-21-style.md`](research-2026-11-21-style.md).
