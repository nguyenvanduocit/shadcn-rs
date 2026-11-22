# shadcn-rs — Nghiên cứu tách style khỏi base

2026-11-21. Tài liệu trả lời một câu hỏi: đưa phần định nghĩa style (look) ra khỏi phần base (cấu trúc và hành vi)
như reka-ui và shadcn làm, theo cách của Rust. Tài liệu ghi số liệu, nguồn, và lý do chọn hay bác bỏ từng phương án.
Quyết định nằm ở [ADR-0022](adr/0022-a-style-is-a-typed-value-components-read.md) (accepted 2026-11-21, chưa xây). Tài liệu này thay hẳn
bản nháp đầu (một hàm look có kiểu cho mỗi part và mỗi style, bốn crate); §9 ghi vì sao bản đó bị bác.

## Mục lục

1. [Tổng quan](#1-tổng-quan)
2. [Hiện trạng trong shadcn-rs](#2-hiện-trạng-trong-shadcn-rs)
3. [Upstream: reka, base, style, theme](#3-upstream-reka-base-style-theme)
4. [Flutter](#4-flutter)
5. [Compose](#5-compose)
6. [Các toolkit khác](#6-các-toolkit-khác)
7. [Thiết kế đích](#7-thiết-kế-đích)
8. [Kiểm chứng](#8-kiểm-chứng)
9. [Đã cân nhắc và bác bỏ](#9-đã-cân-nhắc-và-bác-bỏ)
10. [Kế hoạch](#10-kế-hoạch)
11. [Nguồn](#11-nguồn)

---

## 1. Tổng quan

Hôm nay mỗi component của `crates/shadcn` là một hàm làm ba việc: dựng cấu trúc, gắn hành vi, viết look (màu,
khoảng cách, bo góc, chữ). Look chỉ có một bộ, port từ new-york-v4. Upstream tách thành bốn tầng:
- reka-ui giữ hành vi;
- base giữ cấu trúc;
- style giữ look;
- theme giữ token.

Nhờ vậy một base có tám style và hàng trăm tổ hợp theme.

Câu hỏi thật là **tách cái gì ra**. Upstream tách code vẽ ra stylesheet, vì CSS chỉ cho style đọc cây DOM và
attribute. Rust thì có kiểu, có biến và có hàm, nên thứ được tách ra là **giá trị**. Cách vẽ vẫn là code của component,
viết một lần.

### Nguyên tắc

Người dùng đặt nguyên tắc ngày 2026-11-21, qua ba câu:
1. Một số giới hạn của CSS khiến shadcn phải làm như vậy. Rust là ngôn ngữ biên dịch, nên không cần copy 1:1; cái cần
   là giá trị cốt lõi. "Ví dụ chỗ border, tại sao phải là `border_0`, trong khi có thể là `border(width)`, tương tự cho
   mọi thứ khác."
2. Principle là căn cứ để tự quyết định và đào sâu, "giống như Flutter, Compose Kotlin".
3. Không đem vào bất kỳ engineering nào mà giới hạn cũ của CSS, HTML hay JS đã buộc tác giả shadcn phải làm. "Giờ chúng
   ta có Rust, có GPU, có rất nhiều engineering có thể tốt hơn, cái quan trọng là value, impact." Đừng cố dùng lại class
   name, các phương thức apply hay kế thừa vốn có trong CSS.

Cụ thể:
- **Setter nhận giá trị:** `border(1.0)`, `padding_x(10.0)`, `shape(s)`. Bậc có tên (`sm`, `md`, `xl`) là token của
  theme, không phải tên method.
- **Không mang sang:**
  - class, hook, `@apply` hay bó utility;
  - cascade, specificity, `!`, twMerge;
  - kế thừa thuộc tính của CSS;
  - mọi công cụ parse hay theo dõi CSS của upstream.
- **Style là dữ liệu có kiểu mà component đọc,** như `MaterialTheme` và `ButtonDefaults` của Compose, hay `ThemeData`
  và `VisualDensity` của Flutter.
- **Sự thật mà CSS phải đoán bằng selector là biến Rust hoặc context có kiểu trong component.** Ví dụ: `last:`, `:has()`,
  `group-*`, `peer-*`, `data-[state]`, `[&_svg]`, `dark:`.
- **CSS của upstream là nguồn để đọc giá trị,** như mọi reference khác. Chrome và shadcn-vue chấm kết quả qua verdict,
  nhưng không quyết định cơ chế, và code của ta không đọc CSS.

### Tiêu chí quyết định

Như `docs/research-2026-11-21.md` §1 (AGENTS.md, "How we decide"). Bốn tiêu chí nặng nhất ở đây:

- **Thiết kế tốt nhất ngay.** `Design` làm ở hình dạng cuối, đủ cho tám style. Không làm một style trước rồi mới tính.
- **Không tương thích ngược.** `Styled`, `States`, `Theme` và mọi component đổi trong cùng một thay đổi.
- **Giá trị và impact.** Một cơ chế chỉ được giữ nếu nó cho kết quả tốt hơn: đúng hơn, nhanh hơn, hoặc ít chỗ để sai
  hơn. Upstream có nó thì chưa phải lý do.
- **Chrome là chuẩn tham chiếu.** Verdict chấm từng style.

### Mục tiêu

| Mục tiêu | Hôm nay | Đích | Đo bằng |
| --- | --- | --- | --- |
| Style nằm ngoài component | Mọi component viết giá trị cố định (`button.rs:35-116`) | Component không viết giá trị thị giác cố định nào; mọi giá trị đến từ `Design` hoặc `Theme` | lint `disallowed-methods` trên constructor của `Color` (§8) |
| Hành vi tách khỏi design system | Hành vi nằm trong component và `gui::time_input` | `primitives` không phụ thuộc `shadcn` và không có giá trị thị giác | chiều phụ thuộc của crate |
| Số style | 1 (new-york-v4) | 8: vega, nova, maia, lyra, mira, luma, rhea, sera | build: hằng thiếu field là lỗi compile |
| Verdict | 99/100 trên 389 demo, một style | Mỗi style ≥ 99/100 trên cùng 389 demo | `verdict/run.sh --style <tên>` |
| Khối lượng | — | Mỗi style là một hằng khoảng 250 giá trị; không struct state, không tên part | đếm field |
| Hover một part | `.hover(..)` được áp trong `resolve` lúc layout (`layout/mod.rs`) | chỉ binding paint đọc `hovered` chạy lại (nền; outline và ghost thêm màu chữ), 0 render, 0 byte cấp phát | bộ đếm theo pha trong `FrameReport`, `gui::heap` |
| Đổi sáng/tối | Mọi instance đã đọc `theme()` render lại | 0 render, 0 layout; binding chạy lại ≤ 0,5 ms cho 2.000 part đang mount | bench |
| Đổi style lúc chạy | Không có | 0 render; chỉ binding đọc nhóm đã đổi chạy lại; layout chỉ ở node có thuộc tính layout đổi | bench, devtools |
| Theme | 1 base colour (neutral), sáng/tối | 7 base colour × 17 accent, radius, font sans/heading/mono, menu accent và màu, 5 thư viện icon | verdict trang theme |

### Cách nghiên cứu

- **Code hiện tại.** Đọc `crates/gui` và `crates/shadcn`, đếm chỗ look trộn vào base và chỗ shadcn rò vào `gui`.
- **shadcn.** Đọc shadcn-vue `d1bf1b3` (pin của verdict) và shadcn/ui `45bbf9b` để đối chiếu:
  - pipeline build và CLI;
  - 435 hook của base;
  - histogram variant trong 8 style (18.901 utility);
  - bảng khác biệt theo style;
  - new-york-v4, ví dụ, token.
- **Tám style theo vai.** Đọc tám file style một lần bằng script tạm nằm ngoài repo, rồi bỏ đi. Với mỗi họ (radius,
  chiều cao, padding, chữ, focus, độ nổi, ô nhập, khoảng cách, cấu trúc, motion, màu), thử quy giá trị về vai và đếm
  xem vai giải thích được bao nhiêu (§3.8).
- **Tailwind.** Biên dịch các style bằng Tailwind 4.3.5 trên `verdict/style.css` để chắc mọi rule là CSS hợp lệ.
- **Compose.** Đọc androidx `201ce83`:
  - foundation và material3, 120 file token, Styles API thử nghiệm;
  - `ButtonDefaults`, `ButtonColors`, `TextFieldDefaults`, `Shapes`, `MotionScheme`, `MaterialExpressiveTheme`,
    `CompositionLocal` (§5.8).
- **Flutter.** Đọc `203c0c4`:
  - widgets so với material và cupertino, `ButtonStyle`, `WidgetState`, `ThemeData`, `gen_defaults`;
  - `VisualDensity`, `InputBorder`, `ShapeBorder`, `ThemeExtension`, `InheritedModel`, ink, Cupertino (§4.9).
- **Toolkit khác.**
  - Qt Quick Controls (cả StyleKit), GTK 4, Slint, WinUI 3/WPF, iced, floem, masonry, GPUI, makepad,
    dioxus-components, freya.
  - Ghi đè một khía cạnh cho một instance và cho một cây con được chạy thử trong harness headless của floem và masonry.
- **Trích dẫn.** Mọi `file:line` trỏ vào repo, vào `references/`, hoặc vào upstream ở commit đã ghi.

### Kết luận chính

1. **Thứ tách ra là giá trị, không phải code vẽ.**
   - `Theme` (token) và `Design` (style) là dữ liệu có kiểu.
   - Mỗi component viết cách vẽ một lần, và đọc hai giá trị đó qua defaults công khai của nó, như `ButtonDefaults` của
     Compose đọc `MaterialTheme` (§5.8, §7.6).
2. **Ba crate:** `gui` ← `primitives` ← `shadcn`.
   - `gui` là engine, không biết design system.
   - `primitives` giữ hành vi kiểu reka, không có giá trị thị giác.
   - `shadcn` giữ component, `Theme`, `Design` và tám hằng.
   - Không cần crate style riêng, vì không còn code vẽ nào để tách (§7.1).
3. **Phần lớn khác biệt giữa tám style quy được về giá trị theo vai.**
   - 11 vai radius giải thích 91% số ô.
   - Chiều cao là một thang 4 bậc cho mỗi style, còn padding là một trục riêng.
   - 12 vai chữ khớp 95% số ô. Focus là một ring cho mỗi style.
   - Phần còn lại là riêng của từng component, khoảng một nửa số giá trị, và nằm trong `parts` (§3.8).
4. **Một style là một hằng `Design`,** khoảng 250 giá trị (§7.4, §7.5).
   - Không field nào là `Option`, và không hằng nào xây trên hằng khác.
   - Thêm một field thì tám hằng không compile cho tới khi hằng nào cũng có giá trị.
5. **Màu theo state là dữ liệu cộng một hàm giải:** `ButtonColors`, `Tone { light, dark }`, `Ink`. Thứ tự ưu tiên giữa
   các state là code của component, viết một lần (§7.7).
6. **`gui` nhận giá trị hoặc closure** (§7.2, §7.10, §7.11).
   - Bỏ: `UNIT`, `RADIUS`, `radius_var`, setter đặt tên theo bậc, `has_icon`, refinement của `States`.
   - Thêm: `Interaction` theo node; `Shape`, theo luật góc chồng nhau của CSS; decoration; OKLCH; `ContentStyle`, là
     context tường minh cho chữ và icon, không phải kế thừa.
7. **Ghi đè: mỗi thuộc tính một chủ, ở ba phạm vi.** Không có tầng, không có cascade (§7.8).
   - App: `install`.
   - Cây con: `with_theme`, `with_design`, mỗi cái là computed của giá trị bên ngoài.
   - Instance: tham số có kiểu, hoặc setter thô.
8. **Không render lại khi hover, đổi sáng/tối hay đổi style** (§7.12).
   - `Theme` và `Design` là signal theo nhóm, có equality cutoff, như `InheritedModel` của Flutter.
   - Mỗi binding đi thẳng tới pha của thuộc tính nó gắn.
9. **Vega là style mặc định; new-york-v4 thôi là đích** (§3.7, §8, §3.8.12).
   - Verdict chấm 389 demo dưới từng style, với base Reka chưa mở rộng bên trong `.style-<tên>`. Chrome đọc CSS đó; code
     của ta không đọc.
   - Lỗi của upstream được sửa và ghi vào `better.tsv`.

## 2. Hiện trạng trong shadcn-rs

### 2.1 Ranh giới hôm nay

- **`gui`**: engine (reactive, component, layout, paint, text, input, a11y) cộng một ít hành vi chung:
  - focus và Tab;
  - overlay (`gui::deferred(Placement::…)`);
  - motion (ADR-0007);
  - `time_input.rs`;
  - sửa text.

  Vốn từ style là của Tailwind: `Styled` có 246 setter sinh bằng macro, đặt tên theo class (`styled.rs`).
- **`shadcn`** có 76 module. Mỗi component vừa giữ state và hành vi (ví dụ `AccordionState` với roving focus trong
  `accordion.rs`), vừa dựng cấu trúc, vừa viết look.
- **Quy tắc port** trong `crates/shadcn/README.md`:
  - "Classes translate one for one through `gui::Styled`" (`:21-27`): `rounded-md` → `.rounded_md()`, `hover:` →
    `.hover(|s| …)`, `dark:` → `.when(t.dark, …)`.
  - "Behaviour comes from `gui`, not hand-rolled" (`:31-38`).

### 2.2 Look trộn vào base

- **`button.rs:35-116`**: một hàm làm hết.
  - Cấu trúc: `inline-flex`, căn giữa.
  - Sáu nhánh variant; ba nhánh (destructive, outline, ghost) đọc `t.dark`, và destructive viết cứng `Color::WHITE`
    (`:69`).
  - Tám nhánh size, bốn nhánh có `.has_icon(..)` (`:103-112`).
  - `focus_ring` lặp ở mọi nhánh.
- **`accordion.rs:141`**: "`border-b last:border-b-0`: give the last item `.border_0()`". Người dùng phải biết vị trí
  và tự ghi đè.
- **`theme.rs:8-128`**:
  - `Theme` chỉ có màu và `dark: bool`.
  - `theme()` đọc trong render (`:128`), nên đổi theme làm mọi instance đã đọc nó render lại.
  - Chỉ có một base colour (neutral). Không có radius, font, menu, icon.
- **`bubble.rs:102-106`**: `primary_tint` thành màu xám vì `gui::Color` không lộ thành phần OKLCH. Upstream viết
  `oklch(from var(--primary) l calc(c*k) h)`.
- **`focus.rs:10-16`** (`focus_ring`, `destructive_ring`) và `motion.rs` (animation của popper): look dùng chung nhưng
  nằm cạnh base.

### 2.3 shadcn rò vào gui

| Chỗ | Là gì của shadcn hay Tailwind |
| --- | --- |
| `styled.rs:15` `UNIT = 4.0` | `--spacing` của Tailwind; `p(n)`, `gap(n)`, `h(n)` nhân với nó |
| `styled.rs:19` `RADIUS = 10.0` | `--radius` của theme shadcn |
| `style.rs:103-130` `Radius { times, plus }`, `SM`…`XL3` | `calc(var(--radius) * k)`: thang radius của theme shadcn, giải theo `--radius` kế thừa |
| `styled.rs:329` `radius_var(px)` | `[--radius:…]` |
| `style.rs:542` `FontFamily { Sans, Mono }`, `text/fonts.rs:24-25` | hai font của theme mặc định (Inter, Geist Mono) viết cứng |
| `style.rs:673` icon mặc định 16 px | `[&_svg:not([class*='size-'])]:size-4` của các component |
| `element.rs:62-73` `States` | bảy refinement cố định: `bound`, `has_icon` (giả lập `:has(> svg)`), `hover`, `active`, `focus`, `focus_visible`, `disabled` |
| tên setter trong `styled.rs` | `rounded_md()`, `text_sm()` (14/20), `border_0()`, `tracking_widest()`, `leading_relaxed()`, `shadow_xs()` (cấp phát `vec!` mỗi lần gọi, `:333`) |

`States` được áp trong `resolve` (`layout/mod.rs`). Mỗi lần giải style, hàm này kiểm `state.hovered.contains(path)`
và các tập state khác, không theo node và pha.

## 3. Upstream: reka, base, style, theme

shadcn-vue `d1bf1b3` là pin của `verdict/fetch.sh`. Đường dẫn không có tiền tố là của shadcn-vue (`apps/v4/…`,
`packages/cli/src/…`); `ui:` là của shadcn/ui `45bbf9b`.

### 3.1 Bốn tầng

| Tầng | Ở đâu | Giữ gì |
| --- | --- | --- |
| Hành vi | reka-ui (`references/reka-ui/packages/core/src`, 70 thư mục primitive) | state, bàn phím, focus, a11y; xuất `data-state`, `data-side`, `aria-*` và biến `--reka-*`; không có look |
| Base | `apps/v4/registry/bases/reka/ui` (68 component) | cấu trúc DOM; `data-slot` trên mọi part (399 giá trị tĩnh); 435 hook `cn-*`; utility cấu trúc |
| Style | `apps/v4/registry/styles/style-<tên>.css` (8 file, khoảng 400 rule mỗi file) | `.style-<tên> { .cn-x { @apply …; } }`: look của từng hook |
| Theme | `registry/themes.ts`, `registry/base-colors.ts`, `registry/config.ts` | biến CSS màu (7 base colour, 17 accent), `--radius`, font, menu accent và màu, thư viện icon |

### 3.2 Hợp đồng base–style

- **Hook.** Mỗi phần tử base mang class hook cạnh utility của nó:
  - `cn-<comp>` ở gốc, `cn-<comp>-<part>` ở part;
  - hook do cva chọn từ prop: `-variant-<v>`, `-size-`, `-orientation-`, `-align-`, `-side-`.
- **Mở rộng.** Khi build registry (`apps/v4/scripts/lib/build-styles.ts:83-143`), mỗi hook được thay bằng rule của nó:
  `twMerge(<rule các hook>, <chuỗi gốc bỏ hook>)` (`expand-cn-classes.ts:35-59`).
  - Utility của base đứng sau nên **base thắng** khi trùng.
  - Hook không có rule bị xoá.
  - Năm marker được giữ: `cn-menu-target`, `cn-menu-translucent`, `cn-logical-sides`, `cn-rtl-flip`,
    `cn-font-heading`.
- **Lúc chạy**, trang preview render base chưa mở rộng, và hook khớp rule trực tiếp. Style được import vào
  `layer(base)`, nên utility của component vẫn thắng. Thứ tự ưu tiên giống hệt lúc cài.
- **Style chỉ chạm được thứ base xuất ra**:
  - `data-slot`;
  - attribute ở gốc: `data-variant`, `data-size`, `data-orientation`, `data-side`…;
  - state của primitive, qua custom variant;
  - tên group do base khai báo.
- **Không ai kiểm hợp đồng này**:
  - 55 hook của base không có rule ở style nào (`tabs-list-variant-*`, `field-orientation-*`…); chúng bị xoá lặng lẽ.
  - 32 hook của style không base Reka nào dùng (đó là part của Base UI).
  - `cn-button-group-text` dùng group `button-group`, mà base Vue không khai báo, nên rule không bao giờ khớp.
  - Parser của shadcn/ui gộp `.cn-card-content:has(> [data-slot=questionnaire-choices])` vào `cn-card-content` và mất
    điều kiện `:has` (`ui:apps/v4/scripts/lib/create-style-map.ts:17-61`).

### 3.3 Base giữ gì, style giữ gì

So họ utility trong chuỗi có hook của base với rule của vega cho cùng hook:

| Họ | Base | vega |
| --- | --- | --- |
| layout (display, flex, grid, căn) | 505 (35%) | 91 (4%) |
| kích thước | 205 (14%) | 231 (10%) |
| hành vi (pointer-events, select, cursor, outline-none) | 190 (13%) | 19 (1%) |
| vị trí (relative, absolute, inset, z, translate) | 168 (12%) | 105 (5%) |
| màu | 103 (7%) | 428 (19%) |
| chữ | 61 (4%) | 216 (10%) |
| viền, ring, bóng | 46 (3%) | 322 (15%) |
| motion | 44 (3%) | 209 (9%) |
| khoảng cách | 29 (2%) | 442 (20%) |
| radius | 9 (1%) | 144 (6%) |

- **Base giữ**: display và căn, vị trí của overlay, biến của primitive
  (`max-h-(--reka-select-content-available-height)`), pointer/select/outline, disabled, vùng bấm, khai báo group,
  animation do primitive điều khiển.
- **Style giữ**: màu, khoảng cách, radius, viền, ring, bóng, chữ, chiều cao, animation vào/ra, look theo state.
- **Ngoại lệ: 52 chuỗi base mang look.**
  - Màu của `tabs-list`, màu placeholder của input.
  - `ring-2 ring-background` của avatar-group.
  - Góc nối của button-group.
  - Ring focus của badge, item, scroll-area.
  - `bg-border` của separator. Separator không có hook, nhưng style nào cũng định nghĩa `.cn-separator`.
- **Ngoại lệ: 57 rule của vega mang layout.**
  - `cn-dialog-close` đặt vị trí nút đóng: `absolute top-4 right-4` ở vega, `top-2 right-2` ở nova và lyra,
    `top-5 right-5 bg-secondary` ở sera.
  - `grid` trong `cn-dialog-content` và `cn-alert`.
  - `cn-accordion` chỉ có rule ở maia, luma, mira, rhea (`overflow-hidden rounded-2xl border`), nên bốn style này có
    accordion đóng khung trên cùng một DOM.

### 3.4 Tám style khác nhau ở đâu

- **DOM không khác.** 477 file sinh ra cho mỗi style giống hệt reka-vega, trừ chuỗi literal.
- **Họ thuộc tính khác nhau.** Trong 395 hook mà cả tám style cùng có, 99 hook giống hệt nhau. Số dưới đây là số hook
  khác nhau trên số hook dùng họ đó:
  - radius 121/128, bóng 48/51, chữ hoa 45/45, tracking 40/44;
  - cỡ chữ 119/134, khoảng cách 157/220, kích thước 107/153, độ dày ring 33/53;
  - nền 65/149, màu chữ 20/130, layout/vị trí 14/80.
- **Ví dụ** (các cột sau vega ghi thay đổi so với vega):

| Hook | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `button` | `rounded-md text-sm font-medium`, ring 3 | `rounded-lg` | `rounded-4xl` | `rounded-none text-xs`, ring 1 | `text-xs/relaxed`, ring 2 | `rounded-4xl` | `rounded-2xl` | `rounded-none text-xs font-semibold uppercase tracking-widest`, ring 2, svg 3.5 |
| `button-size-default` | `h-9 gap-1.5 px-2.5` | `h-8` | `px-3` | `h-8` | `h-7 gap-1 px-2` | `px-3` | `h-8 px-3` | `h-10 px-6` |
| `card` | `rounded-xl gap-6 py-6 shadow-xs ring-1 ring-foreground/10` | gap và py 4, không bóng | `rounded-2xl`, không bóng | `rounded-none`, gap và py 4, `text-xs/relaxed` | `rounded-lg`, gap và py 4 | `rounded-4xl shadow-md` | `rounded-[min(var(--radius-4xl),24px)] shadow-sm` | không radius, gap và py 8 |
| `input` | `h-9 rounded-md px-2.5 shadow-xs`, ring 3 | `h-8 rounded-lg`, không bóng | `bg-input/30 rounded-4xl px-3` | `h-8 rounded-none text-xs`, ring 1 | `h-7 px-2 py-0.5 bg-input/20`, ring 2 | `bg-input/50`, viền trong suốt, `rounded-3xl` | `bg-input/50 h-8 rounded-2xl` | chỉ viền dưới, `h-10 px-0`, không radius, không ring |
| `switch` | `rounded-full shadow-xs h-[18.4px] w-8` | không bóng | không bóng | không bóng, ring 1 | ring 2, `h-[16.6px] w-7` | `border-2 h-5 w-11` | `border-2 rounded-2xl h-5` | `rounded-none h-4.5 w-8.25`, ring 2 |

- **Kết luận.** Một style là tập giá trị: radius, chiều cao, padding, cỡ chữ, độ dày ring, bóng, cách dùng màu. Đôi khi
  style chọn thêm cấu trúc trên cùng một cây: accordion đóng khung, input chỉ có viền dưới, vị trí nút đóng. Không style
  nào đổi cây hay hành vi.
- **Phần lớn giá trị theo vai, không theo hook.** Hai ví dụ tưởng là chỉnh tay hoá ra là vai:
  - luma: `select-trigger` là `rounded-3xl`, `button` là `rounded-4xl`. Đó là vai field và vai control, và ở mọi chiều
    cao control cả hai đều vẽ thành stadium.
  - rhea: `min(4xl, 24px)` là vai modal (dialog, alert dialog, command); card cũng lấy giá trị 24 px đó.

  Phần không theo vai là radius của từng bề mặt và hình học riêng của component. §3.8 đếm từng họ.

### 3.5 Theme

- **Màu.** `registry/base-colors.ts` có 7 base colour: neutral, stone, zinc, mauve, olive, mist, taupe.
  `registry/themes.ts` có 24 theme, mỗi theme có bản sáng và tối, `radius: 0.625rem`. `buildRegistryTheme`
  (`registry/config.ts:335-355`) gộp base colour trước, theme sau; CLI ghi kết quả vào `:root` và `.dark` của app.
- **menuAccent.** `bold` đặt `accent`, `accent-foreground`, `sidebar-accent(-foreground)` bằng `primary`,
  `primary-foreground` (`config.ts:345-355`).
- **menuColor.** Bốn giá trị trên hai trục (`registry/config.ts:48-53`): `default`, `default-translucent`, `inverted`,
  `inverted-translucent`.
  - `inverted`: CLI thay `cn-menu-target` bằng class `dark` trong nội dung menu (`transform-menu.ts:17-18`), nên cả cây
    con đọc token tối, ở cả hai scheme của app.
  - `translucent`: dùng rule `.cn-menu-translucent` của style (`style-vega.css:1359`):
    - nền `bg-popover/70`;
    - một lớp `before:` có `backdrop-blur-2xl backdrop-saturate-150`;
    - màu focus của mọi item con là `foreground/10`.
- **radius.**
  - `--radius` cộng thang `--radius-sm…4xl = calc(var(--radius) * k)`, với k = 0,6/0,8/1/1,4/1,8/2,2/2,6
    (`utils/updaters/update-css-vars.ts:549-600`).
  - Trang create ép `none` cho lyra và sera (`composables/useDesignSystemProvider.ts:73-76`).
- **font.** `--font-sans` và `--font-heading`; marker `cn-font-heading` đặt ở các tiêu đề.
- **icon.** Base viết `IconPlaceholder lucide=… tabler=… hugeicons=… phosphor=… remixicon=…`; CLI chọn một thư viện
  (`transform-icons.ts:13-93`).
- **rtl.** `cn-rtl-flip` thành `rtl:rotate-180` và class vật lý thành class logic, chỉ khi `config.rtl`
  (`transform-rtl.ts`).
- **Token được dùng**, tính theo số utility trong 8 style:
  - màu: `destructive` 777, `foreground` 479, `muted` 412, `muted-foreground` 374, `ring` 335, `input` 323;
  - radius: `none` 277, `md` 164, `xl` 123, `lg` 122, `2xl` 122, `full` 117, `4xl` 52, `3xl` 51, `sm` 40;
  - bóng: `md` 62, `lg` 57, `none` 38, `sm` 25, `xs` 19, `xl` 14;
  - chữ: `sm` 524, `xs` 331, `xs/relaxed` 87, `base` 49.
- **Không style nào dùng** `chart-1…5`, `sidebar-primary(-foreground)`, `destructive-foreground`.
- **Màu và số tính từ token.**
  - bubble dùng `oklch(from var(--primary) …)` và `color-mix(in oklch, var(--secondary), var(--foreground) 5%)`.
  - `var(--radius*)` nằm trong `min()` và `calc()`.
  - `--spacing(n)` xuất hiện 74 lần.
  - Một số rule có biến cục bộ: `--card-spacing`, `--cell-size`, `--cell-radius`.

### 3.6 Những gì CSS buộc shadcn phải làm

18.901 utility trong 8 style, 8.390 utility có variant. Mỗi dòng dưới đây là một cách lách giới hạn của CSS:

| Cơ chế | Số lần | CSS cần nó vì |
| --- | --- | --- |
| `data-[k=v]` của chính phần tử | 1.754 | style chỉ đọc được attribute; `data-[side=…]` 726, `data-[slot=…]` 321, `data-[variant=…]` 180, `data-[size=…]` 150 |
| custom variant `data-open`, `data-closed`, `data-checked`… | 1.200 | một style phục vụ hai phương ngữ: `data-state="open"` của Radix/Reka và `data-open` của Base UI (`packages/cli/src/tailwind.css:27-86`) |
| `aria-*` | 514 | state chỉ lộ ra qua attribute |
| `hover:`, `focus:`, `focus-visible:`, `active:`, `disabled:`, `focus-within:` | 456, 412, 456, 43, 28, 55 | pseudo-class |
| `group-*` của tổ tiên; tên group `/name` | 878; 1.009 | style của con không đọc được state của tổ tiên nếu tổ tiên không khai báo group |
| `has-*`, `group-has-*` | 784, 163 | cha không biết con nào có mặt |
| `[&_x]` và `[&>x]`; `*:`; `**:` | 666; 504; 263 | style cho con do app đưa vào, theo kiểu phần tử |
| `peer-*` | 140 | anh em không thấy nhau |
| `in-*` | 88 | biết mình nằm trong part nào (`tooltip-content` 24, `combobox-content` 16) |
| `not-*` | 55 | phủ định selector |
| `dark:` | 548 | scheme đặt bằng class |
| `sm:`, `md:` | 198 | media query theo viewport |
| `before:`, `after:` | 174, 119 | thiếu node phụ: lớp backdrop, viền phủ, vạch hai bên nhãn, dấu `*`, vùng bấm |
| `last:`, `first:` | 56, 31 | vị trí trong anh em |
| `supports-backdrop-filter:` | 32 | dò tính năng |
| `!` (important) | 161 | thắng twMerge hoặc specificity |
| giá trị tuỳ ý `-[…]`, thuộc tính tuỳ ý `[p:v]` | 454, 28 | giá trị nằm ngoài thang |

Không style nào dùng `checked:`, `invalid:`, `placeholder-shown:`, odd/even/nth, `lg:` trở lên, `@container`,
`rtl:`/`ltr:`, `motion-*`.

Ở mỗi dòng, style cần một sự thật mà base đã biết: variant là gì, có icon cuối không, là item cuối không, có nằm trong
button group không, menu mở về phía nào. Nhưng CSS chỉ cho style đọc cây DOM và attribute.

### 3.7 new-york-v4

- **Nội dung.** `registry/new-york-v4/ui` có cùng 69 mục với `bases/reka/ui`. Class viết cứng, không có hook, lucide
  import thẳng, variant dùng `data-[state=open]:` thô.
- **So với các style.** Gần vega nhất (Jaccard theo token: input 0,84, select 0,59, button 0,51…), nhưng không trùng
  style nào.
- **DOM khác base ở vài chỗ.**
  - Accordion: new-york-v4 dùng một `ChevronDown` xoay 180°; base dùng hai icon bật tắt bằng `group-aria-expanded`.
  - Nút đóng dialog: new-york-v4 là `DialogClose` có style riêng; base là `DialogClose as-child` bọc
    `Button variant="ghost" size="icon-sm"`.
- **Vị trí ở upstream.**
  - CLI Vue lấy new-york-v4 làm fallback (`registry/constants.ts:10`); docs site và preview block dùng nó.
  - `init --defaults` chọn nova; khi hỏi, mặc định là vega "(Recommended)" (`init.ts:315,652`).
  - shadcn/ui gọi nó là "the legacy source registry" và từ chối `--style new-york-v4`
    (`ui:apps/v4/registry/README.md:16-18,98`).
- **Trong shadcn-rs.** Ta đang port new-york-v4. Verdict ánh xạ mọi demo viết cho `bases/reka` hay `styles/reka-nova`
  sang component new-york-v4 cùng tên (`verdict/vite.config.ts:19-24`).

### 3.8 Tám style khác nhau theo vai hay theo hook

Câu hỏi: khác biệt giữa tám style có quy được về một ít tham số theo vai (shape của control, chiều cao theo size, kiểu
chữ của nhãn…) không, và phần nào còn lại là riêng của từng component. Câu trả lời quyết định các field của `Design`
(§7.4).

**Cách đọc.**
- Tám file style ở pin `d1bf1b3` được đọc một lần, bằng script tạm nằm ngoài repo, rồi bỏ đi. Không công cụ nào của dự
  án đọc CSS (§9).
- Radius tính bằng px với `--radius` = 10: none 0, xs 2, sm 6, md 8, lg 10, xl 14, 2xl 18, 3xl 22, 4xl 26.
- "Stadium" nghĩa là radius ít nhất bằng nửa chiều cao. CSS thu nhỏ các góc chồng nhau, nên những giá trị đó vẽ ra hình
  viên thuốc. Ở đây đếm theo hình vẽ ra, vì đó là thứ Chrome hiện.

**Số hook.**
- Mỗi style: vega 403, nova 404, maia 404, lyra 399, mira 405, luma 404, rhea 409, sera 400. Hợp lại là 412, và 99
  hook giống hệt nhau ở cả tám style.
- 32 hook của style không base Reka nào dùng. Đó là phần của Base UI: `*-content-logical`, `cn-separator*`,
  `cn-progress-track`…
- 55 hook của base không có rule ở style nào.

#### 3.8.1 Vai giải thích được bao nhiêu

| Họ | Kết quả |
| --- | --- |
| Radius | 7 vai ban đầu giải thích 38/85 hook (78% số ô hook × style, theo hình vẽ ra). Sau khi tách 17 hook là radius riêng của component (§3.8.2), 11 vai tinh chỉnh giải thích 46 trên 68 hook còn lại (91% số ô) |
| Chiều cao | Một thang 4 bậc cho mỗi style khớp đúng 19/29 hook có size. 3 hook lệch ở một style; 7 hook có thang riêng. Padding là một trục thứ hai, độc lập |
| Chữ | 12 vai, nếu bỏ qua line-height, khớp 53/67 hook (95% số ô). Line-height ở lyra và mira thay đổi theo từng component, không theo quy luật nào |
| Focus | Một ring cho mỗi style, cộng luật gạch chân của sera, khớp 18/34 hook có focus. 9 trong 16 hook lệch là do base viết cứng ring |
| Độ nổi | Alpha của ring cộng một bậc bóng cho 4 vai bề mặt: khớp hết card và modal, khớp 7/12 popup |
| Ô nhập | 4 kiểu. Lệch ở ô OTP, ở checkbox và radio của hai style filled, và ở ô nhập sidebar của sera |
| Khoảng cách | Hai mật độ, ánh xạ theo bậc với tỉ lệ 0,5–0,75, không phải một hệ số. Mật độ menu và mật độ bảng là hai trục riêng |
| Cấu trúc | Khoảng 30 rule đổi cấu trúc chứ không đổi giá trị |
| Motion | Giống nhau, trừ danh sách thuộc tính transition, 200 ms của rhea, và overlay drawer của rhea không fade |
| Màu | 95 hook khác nhau ở token màu hay alpha, không tính focus và invalid |

**Vai không giải thích được ở đâu.** Đây là phát hiện quan trọng nhất:
1. **Radius của bề mặt không có vai.** Card, alert, accordion, empty, code block, attachment, bubble, questionnaire
   choice, chart tooltip và toast, mỗi cái chọn một bậc riêng: chỉ 1/9 giải thích được. Card đi theo dialog, không theo
   alert.
2. **Overlay tách làm bốn:**
   - popup: menu, select, combobox, popover, hover card, nav menu;
   - modal: dialog, alert dialog, command;
   - tooltip;
   - toast.

   Popover, hover card và nav menu vẫn lệch khỏi menu ở vega, mira và rhea.
3. **Chip không thành vai.** Avatar tròn ở mọi style, kể cả hai style vuông. Kbd không lần nào khớp badge.
4. **Indicator gần đúng**, với ba ngoại lệ: radio luôn tròn (cả ở lyra và sera), switch của lyra tròn, checkbox có
   radius nhỏ riêng.
5. **Part lồng nhau không lấy giá trị của vai.** Chúng thấp hơn container một bậc: tabs, menubar, menu, nav, command,
   attachment.
6. **Sera đổi cấu trúc chứ không đổi giá trị**, ở khoảng 15 hook.
7. **Base viết cứng focus ở sáu component** (§3.8.5). Trigger của accordion đóng khung không có chỉ báo focus nào.

#### 3.8.2 Radius theo vai

Bảng ghi radius vẽ ra; "→ stadium" nghĩa là bậc được khai báo bị thu thành hình viên thuốc.

| Vai | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control (button, toggle, text trong button group, button trong input group) | md 8 | lg 10 | 4xl → stadium | 0 | md 8 | 4xl → stadium (toggle 3xl) | 2xl → stadium | 0 |
| control nhỏ (sm, xs, icon-sm, icon-xs) | min(md, 10/8 px) = 8 | min(md, 12/10 px) = 8 | stadium | 0 | md; xs là sm 6 | stadium | stadium | 0 |
| field một dòng (input, select, native select, combobox chips, input group, nhóm OTP) | md 8 | lg 10 | 4xl → stadium | 0 | md 8 | 3xl → stadium (input group 4xl) | 2xl → stadium | 0 |
| field nhiều dòng (textarea) | 8 | 10 | xl 14 | 0 | 8 | 2xl 18 | 2xl 18 | 0 |
| item của menu (dropdown, context, menubar, select, combobox) | sm 6 | md 8 | xl 14 | 0 | md 8 | 2xl 18 | xl 14 | 0 |
| nội dung popup (cả trigger và link của nav) | md 8 | lg 10 | 2xl 18 | 0 | lg 10 | 3xl 22 | 2xl 18 | 0 |
| modal (dialog, alert dialog, command) | xl 14 | xl 14 | 4xl 26 | 0 | xl 14 | 4xl 26 | min(4xl, 24 px) = 24 | 0 |
| tooltip | md 8 | md 8 | 2xl 18 | 0 | md 8 | xl 14 | xl 14 | 0 |
| card | xl 14 | xl 14 | 2xl 18 | 0 | lg 10 | 4xl 26 | 24 | 0 |
| alert | lg 10 | lg 10 | lg 10 | 0 | lg 10 | 2xl 18 | 2xl 18 | 0 |
| indicator (switch, slider, progress, thumb của scroll) | full | full | 4xl/full → stadium | 0 (switch full) | md/full → stadium | full | 2xl → stadium | 0 |
| checkbox | 4 px | 4 px | 6 px | 0 | 4 px | 5 px | 5 px | 0 |
| badge | 4xl → stadium | 4xl → stadium | 4xl → stadium | 0 | full | 3xl → stadium | 2xl → stadium | không có hộp |
| kbd | sm 6 | sm 6 | sm 6 | 0 | xs 2 | lg 10 | lg 10 → stadium | 0 |
| radio, avatar | full ở cả tám style | | | | | | | |

- **Stadium là một giá trị.** Góc 3xl và 4xl của luma, góc 2xl đồng đều của rhea, đều vẽ thành stadium ở chiều cao của
  control. Vì vậy bản port phải thu nhỏ radius đúng như CSS: một hệ số chung cho mọi góc.
- **Control nhỏ có trần.** vega và nova viết `min(var(--radius-md), N px)` cho button sm, xs, icon-sm và icon-xs. nova
  viết thêm cho toggle sm và select sm, còn mira cho toggle sm.
  - Ở radius 10, phần px của trần không có tác dụng ở cả hai style (md = 8 px). nova xuống một bậc (lg → md) vì rule
    đọc `--radius-md`, không phải vì trần.
  - Trong button group, giá trị được trả lại: md ở vega, lg ở nova.
- **Radius riêng của từng component.** Mỗi chuỗi ghi theo thứ tự vega/nova/maia/lyra/mira/luma/rhea/sera:
  - empty: lg/xl/lg/0/xl/2xl/3xl/0;
  - code block: lg/lg/xl/0/md/2xl/2xl/0;
  - attachment: xl/xl/2xl/0/lg/3xl/2xl/0;
  - bubble: xl/xl/3xl/0/lg/3xl/3xl/0;
  - questionnaire choice: md/lg/4xl/0/xl/3xl/2xl/0;
  - chart tooltip: lg/lg/lg/0/lg/xl/xl/0;
  - toast: 2xl/2xl/2xl/0/md/2xl/2xl/0;
  - command item: sm/sm/lg/0/md/2xl/xl/0, còn trong dialog thì lg/lg/2xl/0/md/3xl/2xl/0;
  - `cn-item`: md/lg/2xl/0/md/2xl/2xl/0;
  - sidebar menu button: md/md/lg/0/(sm + 2 px)/xl/xl/0;
  - accordion, chỉ ở style đóng khung: maia 2xl, mira md, luma 2xl, rhea 2xl.
- **Overlay lệch khỏi vai popup:**
  - popover: rhea 3xl;
  - hover card: vega lg, rhea 3xl;
  - nav popup: vega lg, mira xl, rhea 3xl.
- **Bằng nhau ở sáu style có bo góc** (lyra và sera đặt 0 cho mọi thứ dưới đây):
  - sheet: 0 (cả tám);
  - table container: 0 (cả tám);
  - mũi tên tooltip: 2 px (`rounded-xs`; rhea viết `rounded-[2px]`, cùng giá trị);
  - tay cầm resizable: lg;
  - tay cầm drawer: full.
- **Không nhất quán ngay trong một style.** Card của maia là 2xl nhưng góc của header, footer và ảnh là xl.

**Góc lồng nhau.** Bảng ghi container > part bên trong, theo bậc; ở lyra và sera mọi cặp là 0 > 0.

| Cặp (padding) | vega | nova | maia | mira | luma | rhea |
| --- | --- | --- | --- | --- | --- | --- |
| nội dung menu > item (p-1; luma 1,5) | md > sm | lg > md | 2xl > xl | lg > md | 3xl > 2xl | 2xl > xl |
| tabs list > trigger (3 px; luma 4 px) | lg > md | lg > md | stadium > xl | lg > md | stadium > stadium | stadium > stadium |
| menubar > trigger (4 px; nova, rhea 3 px) | md > sm | lg > sm | stadium > stadium | lg > (md − 2 px) | stadium > stadium | stadium > stadium |
| command dialog > item (8 px) | xl > lg | xl > lg | 4xl > 2xl | xl > md | 4xl > 3xl | 3xl > 2xl |
| input group > button xs | 8 > 5 | 10 > 7 | stadium > stadium | 8 > 4 | stadium > stadium | stadium > stadium |

- "Bên trong thấp hơn một bậc" đúng ở 31/48 cặp. Có 8 cặp nữa mà cả hai đều là stadium.
- Góc đồng tâm thật sự (radius ngoài trừ padding) chỉ có ở menu của maia và rhea (18 − 4 = 14) và menubar của mira
  (10 − 4 = 6).

**Phạm vi radius cục bộ.**
- Ở maia, luma và rhea, header và content của sidebar đặt `[--radius:var(--radius-xl)]`. Vì vậy mọi radius bên trong
  nhân 1,4, và menu button của luma và rhea thành stadium. Sera đặt `[--radius:0]`.
- Calendar đặt `--cell-radius` và `--cell-size`, nhưng calendar của Reka bỏ qua cả hai. Ô của nó là `size-8` với
  radius của button.

#### 3.8.3 Chiều cao và padding

| | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| chiều cao xs/sm/default/lg | 24/32/36/40 | 24/28/32/36 | 24/32/36/40 | 24/28/32/36 | 20/24/28/32 | 24/32/36/40 | 24/28/32/36 | 28/36/40/44 |
| padding x của button, xs/sm/def/lg | 8/10/10/10 | 8/10/10/10 | 10/12/12/16 | 8/10/10/10 | 8/8/8/10 | 10/12/12/16 | 10/12/12/16 | 12/16/24/32 |
| padding phía có icon | 6/6/8/8 | 6/6/8/8 | 8/8/10/12 | 6/6/8/8 | 6/6/6/8 | 8/8/10/12 | 8/8/10/12 | 8/12/16/20 |
| gap xs, sm / def, lg | 4/6 | 4/6 | 4/6 | 4/6 | 4/4 | 4/6 | 4/6 | 4/6 |
| cỡ icon xs/sm/def/lg | 12/16/16/16 | 12/14/16/16 | 12/16/16/16 | 12/14/16/16 | 10/12/14/16 | 12/16/16/16 | 12/16/16/16 | 12/14/14/14 |

- **Mỗi style một thang.**
  - Ở cả tám style, sm = default − 4 và lg = default + 4. xs là 24, trừ mira (20) và sera (28).
  - Có bốn thang khác nhau: {vega, maia, luma}, {nova, lyra, rhea}, {mira}, {sera}.
- **19 hook khớp đúng thang ở cả tám style:** button (8 size), toggle (3 size), input, input group, select (default và
  sm), native select (default và sm), combobox chips (chiều cao tối thiểu), ô OTP, questionnaire input và actions.
- **Size chỉ có icon** là hình vuông của cùng chiều cao: icon = default, icon-xs = xs, icon-sm = sm, icon-lg = lg. Toggle
  cao bằng button.
- **Padding là trục riêng.**
  - Có bốn họ padding: {vega, nova, lyra}, {maia, luma, rhea}, {mira}, {sera}.
  - Chiều cao và padding đổi độc lập với nhau. Ví dụ: vega cao 36 với padding họ 1; nova cao 32 với padding họ 1; maia
    cao 36 với padding họ 2; rhea cao 32 với padding họ 2.
- **Ngoại lệ:**
  - padding của toggle ở rhea là 10/8, không theo button;
  - padding x của field bằng của button, trừ rhea (10, riêng select 12) và sera (0);
  - tabs list ở mira cao 32; menubar ở mira cao 36;
  - button trong input group có thang riêng;
  - menu button của sidebar: 32/32/36/32/32/36/32/36;
  - table head cao 40 hoặc 48;
  - badge cao 20, kbd cao 20 (luma và sera 22), checkbox và radio 16 (sera 18).

#### 3.8.4 Chữ

Mỗi ô ghi cỡ px, line-height nếu khác mặc định, weight, transform và tracking. M = 500, SB = 600, UC = uppercase,
/r = relaxed.

| Vai | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| nhãn control | 14 M | 14 M | 14 M | 12 M | 12/r M | 14 M | 14 M | 12 SB UC widest |
| field (desktop; hẹp) | 14; 16 | 14; 16 | 14; 16 | 12; 12 | 12/r; 14 | 14; 16 | 14; 16 | 14; 16 |
| item của menu | 14 | 14 | 14 | 12 | 12 (/r) | 14 M | 14 | 12 M UC wider |
| item của list (select, combobox, command, menubar) | 14 | 14 | 14 | 12 | 12/r | 14 M | 14 | 14 |
| body | 14 | 14 | 14 | 12 hoặc 12/r | 12/r | 14 | 14 | 14 |
| tiêu đề (card, dialog, sheet, drawer, popover) | 14 M (card 16) | 16 M | 16 M | 14 M | 14 M | 16 M | 16 M | 18 SB UC wider |
| tiêu đề lớn (alert dialog, empty) | 18 M | 16/14 M | 18 M | 14 M | 14 M | 18 M | 18 M | 18 SB UC wider |
| mô tả (muted) | 14 | 14 | 14 | 12/r | 12/r | 14 | 14 | 14/r |
| label của form | 14/1 M | 14/1 M | 14/1 M | 12/1 (400) | 12/1 M | 14/1 M | 14/1 M | 12/r SB UC wide |
| label của nhóm (menu, select) | 12 M | 12 M | 12 | 12 | 12 | 12 | 12 | 12 SB UC wider |
| badge | 12 M | 12 M | 12 M | 12 M | 10 M | 12 M | 12 M | 10 SB UC widest |

- **Chữ hoa của sera chỉ có ở một phần.** Những chỗ sau không viết hoa:
  - field;
  - item của select, combobox, command, menubar;
  - trigger của menubar và nav (14 M);
  - trigger của accordion (14 SB);
  - tiêu đề alert (14 SB);
  - mô tả.

  Label đứng cạnh checkbox, radio hay switch thành 14, weight 400, không viết hoa.
- **Field ở viewport hẹp** (dưới `md`) là 16 px, để iOS không zoom. Ngoại lệ: mira 14, lyra giữ 12.
- **Ngoại lệ lẻ:**
  - tiêu đề dialog, sheet, drawer và popover của vega không đặt cỡ, nên nhận 14;
  - nova: tiêu đề popover và empty là 14;
  - mọi item của list ở luma là M;
  - shortcut của menu dùng `tracking-widest` ở mọi style;
  - nhãn control đổi theo size: xs là 12 px ở mọi style trừ mira (10 px); sm của nova là 12,8 px.

#### 3.8.5 Focus

| | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| độ dày ring, màu | 3, ring/50 | 3, ring/50 | 3, ring/50 | 1, ring/50 | 2, ring/30 | 3, ring/30 | 3, ring/30 | 2, ring/30 (chỉ control) |

- **Chung ở mọi style:**
  - Khi focus, viền cũng đổi sang màu `ring`. Không style nào dùng offset cho ring.
  - Invalid dùng cùng độ dày ring, màu `destructive/20` (tối: /40), viền `destructive` (tối: /50).
- **Field của sera không có ring.** Chỉ viền dưới đổi màu: sang `ring`, hoặc `destructive` khi invalid.
- **Base viết cứng focus ở nhiều chỗ.** Khi đó base thắng hook, cả trong Chrome của verdict:
  - tabs trigger: 3 px ring/50 cộng `outline-1`;
  - toggle: 3 px;
  - badge và item: 3 px ring/50;
  - viewport của scroll area;
  - tay cầm resizable.
- **Lệch riêng:**
  - radio: lyra và mira dùng 3 px ring/50;
  - thumb của slider: 4 px ở vega, maia, luma, rhea;
  - sidebar: `ring-2 ring-sidebar-ring` (rhea 3);
  - attachment: `focus-within:ring-1`.
- **Item của menu, select, combobox và command** báo focus bằng nền (`bg-accent`; command dùng `bg-muted`).
- **Trigger của accordion đóng khung** (maia, mira, luma, rhea) không có chỉ báo focus: base đặt `outline-none` mà hook
  không đặt ring.

#### 3.8.6 Bề mặt và độ nổi

| | card | popup | sub-menu | dialog, alert dialog | sheet | control |
| --- | --- | --- | --- | --- | --- | --- |
| vega | ring fg/10 + xs | ring fg/10 + md | lg | ring fg/10, không bóng | lg | `shadow-xs` trên field, outline button, toggle, checkbox, switch, menubar, OTP |
| nova, lyra | ring fg/10 | ring fg/10 + md | lg | chỉ ring | lg | không |
| maia | ring fg/10 | ring fg/5 + 2xl | 2xl | ring fg/5 | lg | không |
| mira | ring fg/10 | ring fg/10 + md | md | chỉ ring | lg | không |
| luma | ring fg/5 (tối /10) + md | ring fg/5 (tối /10) + lg | lg | + xl | xl | thumb của switch sm, thumb của slider md |
| rhea | như luma, card bóng sm | + lg | lg | + xl | xl | như luma |
| sera | ring fg/5 + sm | ring fg/10 + md | md | ring fg/10 + md | md | không |

- Overlay dùng ring, không dùng viền, trừ sub-content của context menu ở vega, nova, maia, lyra.
- Tooltip không có ring và không có bóng ở mọi style.
- Tab đang chọn có bóng sm, nhưng chỉ ở vega và nova.

#### 3.8.7 Ô nhập

| | kiểu | viền lúc nghỉ | nền sáng / tối | input px × py | textarea p |
| --- | --- | --- | --- | --- | --- |
| vega | outline | input | không / input/30 | 10 × 4 | 10 × 8 |
| nova, lyra | outline | input | không / input/30; disabled input/50 (tối /80) | 10 × 4 | 10 × 8 |
| maia | outline có nền | input | input/30 / input/30 | 12 × 4 | 12 × 12 |
| mira | outline có nền | input | input/20 / input/30 | 8 × 2 | 8 × 8 |
| luma | filled | trong suốt | input/50 cả hai | 12 × 4 | 12 × 12 |
| rhea | filled | trong suốt | input/50 cả hai | 10 × 4 (select 12) | 10 × 8 |
| sera | gạch chân | viền dưới input, các cạnh khác trong suốt | không | 0 × 4 | 0 × 12 |

- vega có thêm `shadow-xs` trên field.
- Select và native select có thêm `dark:hover:bg-input/50` ở vega, nova, maia, lyra, mira.
- Textarea là `resize-none` ở maia, mira, luma, rhea, sera.
- Ngoại lệ:
  - ô OTP của luma và rhea giữ `border-input`;
  - checkbox và radio của luma và rhea có nền (`bg-input/90`, viền trong suốt);
  - ô OTP của sera là các ô gạch chân rời nhau (`gap-1`).

#### 3.8.8 Khoảng cách của container (px)

| | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| card p và gap (sm) | 24 (16) | 16 (12) | 24 (16) | 16 (12, gap 8) | 16 (12) | 24 (16) | 20 (16) | 32 (20) |
| dialog, alert dialog: p, gap | 24 | 16 | 24 | 16 | 16 (alert dialog gap 12) | 24 | 24 | 24 |
| alert px × py | 16 × 12 | 10 × 8 | 16 × 12 | 10 × 8 | 8 × 6 | 16 × 12 | 16 × 12 | 16 × 12 |
| popover p / gap | 16/16 | 10/10 | 16/16 | 10/10 | 10/16 | 16/16 | 16/16 | 16/16 |
| header và footer của sheet | 16 | 16 | 24 | 16 | 24 | 24 | 24 | 32 |
| empty p | 48 | 24 | 48 | 24 | 24 | 48 | 48 | 48 |
| gap của field group | 28 | 20 | 28 | 20 | 16 | 28 | 24 | 40 |
| nội dung menu p; item px × py | 4; 8 × 6 | 4; 6 × 4 | 4; 12 × 8 | 0; 8 × 8 | 4; 8 × 4 | 6; 12 × 8 | 4; 8 × 6 | 6; 12 × 8 |
| ô bảng p; item px × py | 8; 16 × 14 | 8; 12 × 10 | 12; 16 × 14 | 8; 12 × 10 | 8; 12 × 10 | 12; 16 × 14 | 8; 16 × 14 | 12; 16 × 14 |

- **Không có một hệ số chung.**
  - Ba style gọn (nova, lyra, mira) ánh xạ giá trị thường theo bậc: 48 → 24, 28 → 20, 24 → 16, 16 → 10 (item của bảng:
    16 → 12), 14 → 10, 12 → 8.
  - Ngoại lệ: rhea dùng `--card-spacing` 20; sera có card 32, sheet 32, field group 40; mira có sheet 24.
- **Mật độ menu và mật độ bảng là hai trục riêng.**

#### 3.8.9 Cấu trúc

Khoảng 30 rule đổi cấu trúc thị giác. Tất cả vẫn nằm trên cùng một cây DOM.

- **Accordion đóng khung** ở maia, mira, luma, rhea:
  - root có viền, `overflow-hidden` và radius;
  - item đang mở có nền `muted/50`;
  - trigger có p-4 (mira p-2), gap-6, không radius, không focus;
  - content có px-4.

  Bốn style còn lại là danh sách có vạch chia (`not-last:border-b`; trigger py-4, ở nova và lyra là py-2.5).
- **Drawer.**
  - Nổi ở maia, mira, luma, rhea: nền trong suốt, một lớp `before:` lùi vào 8 px có viền và radius, p-4.
  - Bám mép ở bốn style còn lại: viền ở cạnh trong, radius ở cạnh trong.
- **Thanh footer.**
  - nova: footer của dialog và alert dialog tràn ra mép (`-mx-4 -mb-4 border-t bg-muted/50 p-4`). Footer của card là
    `border-t bg-muted/50 p-4`, và card bỏ padding dưới.
  - lyra: footer của card là `border-t p-4`.
- **Alert của sera:** vuông, có một vạch nhấn 2 px ở cạnh trái (`after:`, màu `foreground` hoặc `destructive`), nền
  `background`.
- **Badge của sera:** không viền, không padding, không nền, không chiều cao. Nó là một nhãn chữ.
- **Menu sát mép của lyra:**
  - nội dung của dropdown, context, menubar, select và combobox có p-0;
  - separator bỏ margin dọc;
  - input của command dùng `border-b` thay cho p-1.
- **Nút đóng.**
  - Dialog: 16 px (vega, maia, luma, rhea), 8 px (nova, lyra, mira), 20 px (sera).
  - Sheet: 16 px, riêng nova và lyra là 12 px.
  - luma, rhea và sera tô nền `bg-secondary` cho cả hai nút.
- **Ô OTP của sera** là các ô rời nhau (`gap-1`). Mỗi ô có viền 1 px đủ bốn cạnh, nhưng chỉ cạnh dưới có màu
  (`border-transparent border-b-input`), thay vì các ô nối nhau (`border-y border-r first:border-l`).
- **Offset tuyệt đối khác nhau:**
  - action của alert;
  - action của code block;
  - icon của native select;
  - indicator của item trong menubar;
  - action của sidebar menu;
  - mũi tên tooltip.
- **Khác:**
  - vùng bấm của thumb slider (`after:-inset-2`) ở nova, lyra, mira;
  - bề rộng của hover card (256 hoặc 288) và bề rộng tối thiểu của menu (128 hoặc 192);
  - `max-w` của dialog và alert dialog;
  - sera thêm `mt-0.5` cho sáu hook mô tả.
- **Không style nào đổi** grid, thứ tự, hiển thị hay overflow, ngoài accordion đóng khung.

#### 3.8.10 Hình học riêng của component

| | vega | nova | maia | lyra | mira | luma | rhea | sera |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| track của switch h × w (sm) | 18,4 × 32 (14 × 24) | như vega | như vega | như vega | 16,6 × 28 (14 × 24) | 20 × 44, border-2 (16 × 28) | 20 × 32, border-2 (16 × 24) | 18 × 33 (14 × 25) |
| thumb của switch (sm) | 16 (12) | như vega | như vega | như vega | 14 (12) | 16 × 24 (12 × 16) | 16 (12) | 14 (10), vuông |
| checkbox, radio | 16, r4 | 16, r4 | 16, r6 | 16, r0 | 16, r4 | 16, r5, có nền | 16, r5, có nền | 18, r0; radio là một vòng viền |
| track / thumb của slider | 6 / 16 trắng, viền primary, bóng sm | 4 / 12 | 12 / 16 | 4 / 12 vuông | 4 / 12 | 8 / 24 × 16 trắng, md | 4 / 16 trắng, md | 2 / 12 vuông, nền primary |
| chiều cao progress | 6 | 4 | 12 | 4 | 4 | 12 | 8 | 2 |
| calendar p | 12 | 8 | 12 | 8 | 12 | 12 | 12 | 12 |
| kbd h / px / radius / chữ | 20/4/6/12 | như vega | như vega | 20/4/0/12 | 20/4/2/10 | 22/6/10/12 | 20/4/10/12 | 22/6/0/12 |
| media của alert dialog / glyph | 64/32 | 40/24 | 64/32 | 40/24 | 32/16 | 64/32 | 64/32 | 64/32 |
| icon media của empty / glyph | 40/24 | 32/16 | 40/24 | 32/16 | 32/16 | 40/20 | 40/20 | 40/20 |
| tay cầm drawer | 6 | 4 | 6 | 4, vuông | 6 | 6 | 6 | 6, vuông |

Bằng nhau ở mọi style: avatar (32; sm 24, lg 40; tròn), separator, bề rộng scrollbar 10, kích thước tay cầm
resizable.

#### 3.8.11 Motion và màu

- **Motion giống nhau ở mọi style.** Popup và dialog dùng fade, zoom 95 và slide 2 trong 100 ms. Sheet slide 10 trong
  200 ms. Viewport của nav zoom 90.
- **Thuộc tính transition của field khác nhau:**
  - vega: màu chữ (`color`) và bóng, nên ring mờ dần vào, còn viền và nền đổi ngay;
  - nova, maia, lyra, mira: mọi thuộc tính màu (`transition-colors`) nhưng không có bóng, nên ring hiện ngay;
  - luma: như vega, cộng màu nền;
  - rhea: như vega, trong **200 ms**;
  - sera: màu chữ và màu viền.

  Overlay drawer của rhea không fade.
- **Màu khác nhau theo vai** (95 hook):
  - **outline button:** `background` hay trong suốt; tối `input/30`.
  - **hover của button default:** `primary/80`, nhưng nova và lyra chỉ áp cho `<a>`, nên button thường không có
    hover. Hover của secondary ở rhea là `color-mix(secondary, foreground 5%)`.
  - **ring của overlay:** fg/10 hay fg/5.
  - **backdrop:** black/10 + blur-xs, black/80, black/30 + blur-sm, hay black/20.
  - **separator của menu:** `border` hay `border/50`.
  - **indicator chưa chọn:** viền `input`, hoặc nền `input/90` (luma, rhea).
  - **switch tắt, track của slider, chip, trạng thái đã chọn:** mỗi thứ có hai đến bốn biến thể.
  - **nút đóng:** nền `secondary` ở luma, rhea, sera.

#### 3.8.12 Lỗi của upstream

Đọc dữ liệu cũng lộ ra vài lỗi. Ta sửa, không port, và ghi vào `better.tsv` (§8):
- `cn-menubar-checkbox-item` của lyra có `pr-28` (112 px).
- Variant outline của sidebar ở rhea bọc token oklch trong `hsl(var(--sidebar-border))`. Giá trị đó không hợp lệ, nên
  bóng outline bị bỏ.
- Item của select và combobox ở mira thiếu `pr-8` và cỡ indicator, nên dấu check có thể đè lên chữ.
- Trigger của accordion đóng khung không có chỉ báo focus. Đây là một lỗi a11y; ta dùng ring focus của style.

#### 3.8.13 Hệ quả cho thiết kế

Dữ liệu cho thấy một style là ba loại giá trị:
1. **Thang và vai**, giải thích phần lớn số ô:
   - thang chiều cao 4 bậc;
   - thang padding độc lập với chiều cao;
   - 11 vai góc (§3.8.1), trên luật thu nhỏ góc của CSS. Vai mà chỉ một component dùng (card, alert, tooltip,
     checkbox, badge, kbd) nằm trong `parts` của component đó (§7.4);
   - 12 vai chữ, cộng cỡ chữ của field ở viewport hẹp;
   - ring (độ dày, alpha) và cách field báo focus;
   - ring và bóng theo bề mặt;
   - kiểu ô nhập;
   - mật độ của container, menu và bảng.
2. **Lựa chọn cấu trúc**, mỗi cái là một enum có dữ liệu:
   - accordion đóng khung;
   - drawer nổi;
   - thanh footer;
   - alert có vạch nhấn;
   - badge chữ;
   - menu sát mép;
   - nút đóng có nền và offset;
   - ô OTP liền hay rời.
3. **Giá trị riêng của component** mà vai không giải thích được: radius của từng bề mặt, hình học ở §3.8.10, các offset
   tuyệt đối. Khoảng một nửa số giá trị của một style thuộc loại này.

Quy ước "part bên trong thấp hơn container một bậc" đúng ở phần lớn các cặp. Nó là cách viết hằng cho nhanh, không phải
một luật lúc chạy: ngoại lệ có thật, nên mỗi góc lồng vẫn là một giá trị của riêng nó.

## 4. Flutter

`references/flutter` ở `203c0c4` (2026-11-20). Tiền tố đường dẫn:
- W/, M/, C/, R/ là `packages/flutter/lib/src/{widgets,material,cupertino,rendering}`;
- GD/ là `dev/tools/gen_defaults`.

### 4.1 Tầng

- **`widgets` không có look.**
  - GestureDetector, Focus, Actions/Shortcuts, Scrollable.
  - `EditableText`: "a low-level widget… building block for custom widget sets" (W/editable_text.dart:619-626). Style
    và màu con trỏ là tham số `required`, không có mặc định.
  - RawAutocomplete, RawMenuAnchor, RawRadio, RawTooltip, Expansible, Overlay/OverlayPortal.
- **Chiều import.** Không file nào trong `widgets` import một design library; không file cupertino nào import material.
- **Material và Cupertino đang rời khỏi SDK.**
  - `lib/material.dart:17-22` đã deprecated, thay bằng `package:material_ui`.
  - Lý do trong hướng dẫn chuyển: "Style-neutral core: Decoupling lays the groundwork for a style-neutral Flutter core
    widget catalog".
  - Issue #53059 liệt kê những gì đã đẩy xuống raw widget (RawMenuAnchor, RawRadio, RawTooltip, Expansible,
    showRawDialog). Nó cũng liệt kê, kèm lý do, những gì ở lại Material, ví dụ InkWell: "Relies on Material ancestor to
    paint".

### 4.2 `ButtonStyle` và `WidgetStateProperty`

- **Schema.** `ButtonStyle` có 25 field, 16 field là `WidgetStateProperty<T>`, mọi field mặc định null
  (M/button_style.dart:51, 199-427).
- **Giải từng field, từng state:** `widgetValue ?? themeValue ?? defaultValue` (M/button_style_button.dart:382-393).
  Null nghĩa là nhường tầng dưới.
- **State là kiểu chung.** `WidgetState` (hovered, focused, pressed, dragged, selected, scrolledUnder, disabled, error)
  được chuyển từ Material xuống Widgets ở bản 3.22, vì nó "useful outside the Material library"
  (M/material_state.dart:63-68). Primitive giữ tập state qua `WidgetStatesController`, và chỉ báo khi tập thay đổi
  (W/widget_state.dart:1146-1151).
- **Pha.**
  - Đổi state thì gọi `setState` và giải lại mọi field ("Force a rebuild to resolve WidgetStateProperty properties",
    button_style_button.dart:324-327).
  - Sau đó setter của render object chọn pha: đổi màu chỉ đánh dấu paint (R/proxy_box.dart:2117), đổi padding đánh dấu
    layout (R/shifted_box.dart:142-144).
- **Defaults cùng kiểu với override**: `_FilledButtonDefaultsM3 extends ButtonStyle` (M/filled_button.dart:524-663).
- **`styleFrom`** dựng lại các màu theo state từ màu nền, vì "other colors… depend on the foregroundColor" (design doc
  button, 2020).

### 4.3 Theme

- **`ThemeData`** có 89 tham số và 46 theme component (M/theme_data.dart:265-383). Năm 2015 có 5 tham số, năm 2020
  có 64.
- **Quy ước chuẩn hoá** (flutter.dev/go/material-theme-system-updates, 2020):
  - `FooThemeData` có mọi field null.
  - `FooTheme.of(context)` lấy theme gần nhất, không có thì lấy `Theme.of(context).fooTheme`.
  - Widget tính mặc định theo thứ tự: prop, theme component, `colorScheme` và `textTheme`, mặc định nội bộ.
  - Lý do: "The Interdependent ThemeData Problem".
- **`ThemeExtension<T>`** (`copyWith`, `lerp`) chở token riêng của app, tra theo kiểu (M/theme_data.dart:131-146, :993).
- **Phụ thuộc cả object.**
  - `Theme.of` phụ thuộc toàn bộ `ThemeData`, nên đổi một field làm mọi widget đọc theme build lại.
  - Issue #134065 (phụ thuộc theo từng khía cạnh) vẫn mở.
  - Issue #102993: "Theme's scope is so large, that changing one thing will likely cause [many] widgets to reload
    anyway".
- **`AnimatedTheme`** nội suy bằng `ThemeData.lerp`, lần qua mọi theme component.

### 4.4 Defaults sinh từ token

- `gen_defaults` đọc 62 file JSON: 2.317 token chia ba tầng `md.ref`, `md.sys`, `md.comp`. Nó ghi vào khối
  `// BEGIN GENERATED TOKEN PROPERTIES` trong file component (GD/lib/template.dart:82).
- **Màu vẫn đọc từ theme lúc chạy** (`_colors.primary`). **Hình học và opacity được biên dịch thành hằng**: shape là
  `const`, size và elevation là literal (GD/lib/template.dart:133-226).
- Công cụ này đã bị đánh dấu "legacy" (GD/README.md).

### 4.5 Style kế thừa

- `DefaultTextStyle` (W/text.dart:55): `Text` gộp style mặc định khi `inherit` bật.
- `IconTheme` (W/icon_theme.dart:24): `Icon` lấy `size ?? iconTheme.size`.
- Button đặt màu chữ qua `Material.textStyle`, màu và cỡ icon qua `IconTheme` (button_style_button.dart:550-554,601).
- ListTile bọc slot của nó trong `IconTheme.merge` (M/list_tile.dart:1008-1012).
- Quy tắc của style guide: "Widgets should be entirely agnostic about the type of that child. Don't use `is`…"
  (Style-guide-for-Flutter-repo.md:153). Container tạo style cho nội dung app đưa vào bằng ambient, không soi con.

### 4.6 Cấu trúc không cần selector

- **Part tuỳ chọn là slot `Widget?` có tên**; có mặt hay không là một phép kiểm null.
  - ListTile chọn chiều cao 88, 72 hay 56 theo `(isThreeLine, subtitle != null)` (M/list_tile.dart:1493-1501).
  - InputDecorator có 11 slot, khoảng cách tuỳ `prefixIcon != null` (M/input_decorator.dart:1222-1228).
- **Vị trí do cha tính.**
  - ToggleButtons tính `_isFirstButton`/`_isLastButton` và chỉ bo góc ngoài (M/toggle_buttons.dart:462-532).
  - `ListTile.divideTiles` là `last:border-0` viết thành biến đổi danh sách (M/list_tile.dart:782-805).
  - SegmentedButton vẽ vạch chia và viền ngoài trong render object của nó (M/segmented_button.dart:1061-1150).

### 4.7 Hai design system trên một base

- **Cupertino dùng lại raw widget.** CupertinoScrollbar kế thừa RawScrollbar, CupertinoTextField dựng EditableText,
  CupertinoRadio dựng RawRadio, menu dùng RawMenuAnchor.
- **Cây widget của hai bên khác nhau.** CupertinoButton là MouseRegion → FocusableActionDetector → RawGestureDetector
  → Semantics → FadeTransition. Material và Cupertino là hai bộ component, không phải hai style trên một cấu trúc như
  shadcn.
- **Theme của Cupertino** có 8 field và không có theme theo component. Màu được giải theo môi trường (sáng/tối,
  interface level, high contrast), không theo state tương tác.

### 4.8 Bài học

- **Bão rebuild.**
  - Closure do `styleFrom` tạo ra làm phép so `ThemeData ==` sai. Kết quả: mỗi lần rebuild, mọi widget đọc
    `Theme.of` đều nhận thông báo (#89127).
  - `WidgetStateProperty.resolveWith` vẫn không so sánh được (PR #154695).
- **Schema cố định không theo kịp.** `backgroundBuilder` và `foregroundBuilder` phải thêm năm 2024 để vẽ gradient
  (#89563, #130335); PR bị revert rồi land lại.
- **`styleFrom` trôi khỏi `ButtonStyle`.** PR #154821 sửa chỗ thiếu prop icon.
- **`ThemeData` phình từ 5 lên 89 tham số.** Vài theme component vẫn có kiểu `Object?` và được kiểm lúc chạy
  (theme_data.dart:389-397).

### 4.9 Biến thể thị giác là giá trị có kiểu

P/ là `packages/flutter/lib/src/painting`.

- **Mật độ là hai con số.**
  - `VisualDensity` có `horizontal` và `vertical`, mỗi số trong khoảng ±4 (M/theme_data.dart:3183-3198). Mỗi bậc là 4 px
    (`baseSizeAdjustment`, :3307-3314).
  - Có ba preset: standard, comfortable, compact (:3205-3225). Trên desktop, mặc định là compact (:3252-3257).
  - Button cộng độ lệch này vào kích thước tối thiểu, vào padding và vào vùng bấm (button_style_button.dart:456-505,
    579-584).
- **Kiểu ô nhập là một giá trị.**
  - `InputDecoration` có các field `filled`, `fillColor`, `border`, `isDense`, `contentPadding`
    (M/input_decorator.dart:3313-3844).
  - `border` là một `InputBorder` (M/input_border.dart:34):
    - `UnderlineInputBorder` vẽ một đường ở đáy (:143, :239-274);
    - `OutlineInputBorder` vẽ cả khung và chừa khe cho label (:309, :534-559);
    - `ShapedInputBorder` bọc một `ShapeBorder` bất kỳ (:606).
  - Padding mặc định rẽ nhánh theo ba giá trị đó: `isOutline`, `filled`, `isDense` (input_decorator.dart:2588-2620).
    Không có bộ rule nào cho từng kiểu ô.
- **Shape là một kiểu, vẽ được và nội suy được.**
  - `ShapeBorder` có `getOuterPath` (P/borders.dart:542), `paint` (:667), `lerpFrom` và `lerpTo` (:465, :497).
  - Các shape có sẵn: `RoundedRectangleBorder`, `StadiumBorder`, `RoundedSuperellipseBorder`
    (P/rounded_rectangle_border.dart:240, vẽ bằng `drawRSuperellipse` :339), `ContinuousRectangleBorder`,
    `CircleBorder(eccentricity)`, `BeveledRectangleBorder`, `StarBorder`, `LinearBorder`.
  - `StadiumBorder` nội suy được từ hình tròn và từ chữ nhật bo góc (P/stadium_border.dart:36-55), nên đổi shape cũng
    animate được.
- **Dữ liệu theme của app cũng có kiểu.** `ThemeExtension<T>` có `copyWith` và `lerp`, và được tra theo kiểu:
  `Theme.of(context).extension<T>()` (M/theme_data.dart:131-146, :986-993).
- **Chỉ MediaQuery phụ thuộc theo khía cạnh.**
  - `InheritedModel` cho widget chỉ phụ thuộc vào một khía cạnh (W/inherited_model.dart:121-192). Ví dụ,
    `MediaQuery.sizeOf` chỉ build lại khi kích thước đổi (W/media_query.dart:1667-1676).
  - `Theme.of` thì phụ thuộc cả object `ThemeData` (M/theme.dart:136-138, :245).
- **State đổi thì build lại.** `ButtonStyleButton` gọi `setState` mỗi khi tập state đổi (button_style_button.dart:324-327).
  Sau đó nó giải từng thuộc tính theo `widget ?? theme ?? default`, với state đã giải (:378-393).
- **Ink chỉ vẽ lại.** `InkWell` nhờ `Material` tổ tiên vẽ splash (M/material.dart:563-636). Animation của splash chỉ gọi
  `markNeedsPaint`, không build lại (M/ink_splash.dart:152-163).
- **Cupertino.**
  - Variant là named constructor (`CupertinoButton.tinted`, `.filled`, C/button.dart:73-141) trên một enum riêng,
    cộng thêm `sizeStyle`.
  - Hàm build đọc theme trực tiếp (:467-488), rồi bọc con trong `DefaultTextStyle` và `IconTheme` (:595-598).
  - `CupertinoThemeData` có 8 field. Nó không có shape, density, motion hay extension (C/theme.dart:175-523).

### 4.10 Lấy gì, bỏ gì

| Của Flutter | Ở đây | Lý do |
| --- | --- | --- |
| Tầng hành vi không có look; design library phụ thuộc nó, không bao giờ ngược lại | crate `primitives` | Flutter tách ra để có "style-neutral core". Ta cần tầng này cho tám style và cho design system khác |
| `WidgetState` có kiểu, do primitive giữ | `Interaction` theo node, cộng sự thật có kiểu của primitive | Hover, press, focus chỉ có một nguồn sự thật |
| `widget ?? theme ?? defaults`, giải theo từng thuộc tính | tham số của instance; thiếu thì dùng mặc định của component, đọc `Design` của cây con (§7.8) | Cùng thứ tự ưu tiên. Mỗi thuộc tính có một chủ, nên không cần field null |
| `VisualDensity`, `InputBorder` cộng `filled`, các `ShapeBorder` | field có kiểu của `Design`: chiều cao theo size, `FieldKind::{Outline, Filled, Underline}`, `Corner`, `gui::Shape` | Biến thể là giá trị, và mỗi biến thể chỉ được đọc ở một chỗ |
| `ThemeExtension` | không cần: style riêng của app là một giá trị `Design` | `Design` là dữ liệu của ta, nên thêm field là thêm vào chính nó (không cần tương thích ngược) |
| `DefaultTextStyle`, `IconTheme` | `ContentStyle` của `gui`: container cung cấp, chỉ text và icon đọc (§7.10) | Container không phải soi con. Ngoài text và icon, chỉ `Direction` (ADR-0004) chảy xuống cây, vì layout đọc nó ở mọi node |
| Slot có tên, vị trí do cha tính | slot có kiểu, `Place` | Có mặt hay không, đứng ở đâu, đều đã biết lúc render |
| `InheritedModel` theo khía cạnh | `Theme` và `Design` là signal theo nhóm (§7.12) | Đổi sáng/tối thì binding hình học không chạy lại |
| Ink chỉ vẽ lại | binding ở pha paint | Hover chỉ ghi lại paint chunk của node đó |
| Bỏ: build lại khi state đổi | — | Binding đọc `Interaction` chỉ chạy lại ở pha của nó |
| Bỏ: `Theme.of` phụ thuộc cả object | — | Signal theo nhóm |
| Bỏ: `ButtonStyle` với mọi field nullable, `styleFrom` | — | Đó là object style "túi" mà hướng dẫn của Compose khuyên tránh (§5.8). Null nghĩa là "nhường tầng dưới", nên đọc code không biết ai thắng |
| Bỏ: hình học biên dịch thành hằng (`gen_defaults`) | — | Flutter chỉ có một design language cho mỗi thư viện. Ta có tám style trên một cài đặt, đổi được lúc chạy, nên hình học phải là dữ liệu đọc lúc chạy |

## 5. Compose

androidx `201ce83` (2026-11-20). Tiền tố đường dẫn:
- F/ là `compose/foundation/foundation/src/commonMain/kotlin/androidx/compose/foundation/`;
- M/ là `compose/material3/material3/src/commonMain/kotlin/androidx/compose/material3/`;
- G là `compose/docs/compose-component-api-guidelines.md`.

### 5.1 Tầng và Button

- **Bốn tầng:**
  - runtime;
  - ui: layout, vẽ, input, semantics, Modifier.Node;
  - foundation: `clickable`, `hoverable`, `focusable`, `selectable`, `toggleable`, `InteractionSource`,
    `IndicationNodeFactory`, `BasicText`, `BasicTextField`, lazy list;
  - material3: theme, token, ripple, Surface, `*Defaults`, `*Colors`.
- **Quy ước tên:** `Basic*` nghĩa là "no design-system based visual opinions" (G:263).
- **Button** (M/Button.kt:138-182): `colors.containerColor(enabled)` → `Surface(shape, color, contentColor)` →
  `ProvideContentColorTextStyle(contentColor, typography.labelLarge)` → `Row(defaultMinSize, padding(contentPadding))`.
  - Surface cung cấp `LocalContentColor`.
  - `Text` lấy màu theo thứ tự `color`, rồi `style.color`, rồi `LocalContentColor.current` (M/Text.kt:125).
- **Ripple là một node.** Nó đọc `currentValueOf(LocalContentColor)` lúc vẽ (M/Ripple.kt:773-786). Indication tự thu
  interaction và chỉ gọi `invalidateDraw()`, không recompose (F/Indication.kt:213-273).

### 5.2 Token

- **120 file** trong `M/tokens/`, khoảng 2.670 giá trị, ghi "GENERATED CODE - DO NOT MODIFY BY HAND".
- **Ba tầng:**
  - reference: palette;
  - system: 48 vai màu dạng `ColorToken(id)`, 15 shape, 30 typography, elevation, opacity theo state;
  - component: ví dụ `FilledButtonTokens.ContainerColor = ColorSchemeKeyTokens.Primary`.
- **Giải lúc compose:** `ColorToken.value` gọi `MaterialTheme.colorScheme.fromToken(id)` (M/ColorScheme.kt:1487,1550).
- Chỉ 15 trong 35 token của `FilledButtonTokens` được đọc.

### 5.3 Theme

- **Theme theo cây con.** Mọi tham số của `MaterialTheme` mặc định là giá trị hiện tại, nên mỗi phần UI có thể có theme
  riêng (M/MaterialTheme.kt:33-36).
- **M3: một object bất biến.** Cả theme nằm trong một `staticCompositionLocalOf`; đổi nó là recompose toàn bộ nội dung.
  Để bù, `ColorScheme` cache 65 object màu mặc định (M/ColorScheme.kt:638).
- **M2: một state cho mỗi màu.** Lý do: thay cả object "causes every composable consuming LocalColors to recompose,
  which is prohibitively expensive in cases such as animating one color" (compose/material/…/Colors.kt:319-326).
- **Hướng dẫn:** chỉ đọc theme trong biểu thức mặc định của tham số, "so users can override them" (G:446-448).

### 5.4 Pha

- Trang phases của Compose: framework theo dõi state được đọc trong từng pha và chỉ báo pha đó. Lời khuyên: "Defer
  reads as long as possible".
- Hướng dẫn API: nhận `() -> T` thay vì `T`, vì "if the value is only read during drawing operation, only redraw will
  occur" (G:815-839). `LinearProgressIndicator(progress: Float)` bị deprecated, thay bằng `progress: () -> Float`.
- `currentValueOf(local)` gọi trong measure, draw hay `observeReads` được theo dõi theo pha
  (compose/ui/…/CompositionLocalConsumerModifierNode.kt:41-58).

### 5.5 Slot thay cho `:has()`

- **Slot nullable**, kiểm lúc compose: `supportingContent?.let` (M/ListItem.kt:141-230).
- **Tham số mặc định tính từ slot:** `contentPaddingFor(buttonSize, hasStartIcon = icon != null)`
  (M/ToggleButton.kt:170-171). Đây chính là `has-[>svg]:px-*` của shadcn.
- **Đo lúc layout:** ListItem phát hiện text nhiều dòng bằng baseline đầu và cuối, rồi chọn chiều cao
  (M/ListItem.kt:879-1000).

### 5.6 Styles API: thử nghiệm, đang viết lại

- **API 1.11 đã deprecated.** API đó gồm `Style`, `StyleScope`, `Modifier.styleable` và khoảng 60 thuộc tính có sẵn.
  Lời giải thích: "The style API is moving to a model that allows the design system using it to define the properties
  used in its style parameter" (F/style/StyleModifier.kt:146-160). Material3 chưa dùng API này.
- **Thuộc tính do design system định nghĩa.** `stylePropertyOf(name, interpolate) { default }` tạo một thuộc tính;
  `styleLocalOf` tạo thuộc tính kế thừa xuống cây (F/style/StyleProperty.kt:67-104).
- **Kiểu style theo component.** `CustomStyle<ScopeT>`: design system có một kiểu chung và các kiểu riêng của từng
  component. Mỗi kiểu chỉ lộ "the properties, states, and subcomponents a design system, and its components,
  supports" (CustomStyle.kt:42-50).
- **State là khoá có kiểu.**
  - `StyleState` có enabled, focused, hovered, pressed, selected, checked, `triStateToggle`, cộng `StyleStateKey<T>`
    tuỳ ý (StyleState.kt:231-344).
  - Interaction tự đổ vào state; component ghi phần còn lại (StyleState.kt:581-652).
- **Lớp ưu tiên.**
  - Khối state thắng giá trị gốc bất kể thứ tự: `{customFloat(10f); styleLayer{customFloat(20f)}; customFloat(30f)}`
    ra 20 (StyleCollectorTest.kt:108-118).
  - Style mặc định của component đứng trước, style của người gọi đứng sau (`baseButtonStyle then style`).
- **Part lồng nhau.** Cha gọi `provideNestedStyle(key, style)`, con gọi `applyNestedStyle(key)`. Điều kiện của cha
  được giải theo state của cha (NestedStyles.kt:26-65, StylePropertyCollector.kt:135-176).
- **Giải ngoài composition, đọc ở pha tiêu thụ.**
  - Style chạy trong `observeReads` của một Modifier.Node, và mỗi giá trị là một state.
  - `Modifier.appearance{}` đọc nền, viền, bóng lúc vẽ và chỉ gọi `invalidateDraw()`; `placement{}` đọc lúc measure
    (StyleResolverNode.kt:196-208, Appearance.kt:48-64, Placement.kt:166).
  - Bản cũ dùng bảng thuộc tính → pha (InnerLayout, OuterLayout, Draw, Layer, TextLayout, TextDraw) cộng một phép diff
    (StyleableProperties.kt:53-58).
- **Animation khai báo.** `animate(toSpec, fromSpec){…}`: tác giả chỉ ghi giá trị theo state. Animation ngắt được giữa
  chừng và chạy tiếp từ giá trị hiện tại (StyleAnimations.kt:38-73).

### 5.7 Bài học

- "Avoid grab-bag style parameters and classes, akin to `ComponentStyle`" (G:335-410). Class `*Colors` theo một mục
  đích thì được phép.
- **Mỗi state hay vai mới buộc một deprecation:** `ListItemColors` (selected, dragged), `MenuItemColors`, các overload
  ẩn của `ColorScheme`, `outlinedButtonBorder(enabled)`.
- **Material vi phạm hướng dẫn của chính nó:** Button đọc `labelLarge` và `MinWidth` trong thân hàm, và đọc trạng thái
  nhấn lúc compose (Button.kt:155, 169-175, 256).

### 5.8 Defaults, màu theo state, scheme

- **Button nối tham số vào Surface.**
  - Tham số (M/Button.kt:139-150): `shape = ButtonDefaults.shape`, `colors = ButtonDefaults.buttonColors()`, `border`,
    `contentPadding`, `interactionSource`.
  - Thân hàm (:152-180):
    - giải `colors.containerColor(enabled)`;
    - gọi `Surface(shape, color, contentColor, border, shadowElevation)`;
    - gọi `ProvideContentColorTextStyle(contentColor, typography.labelLarge)`;
    - đặt nội dung trong `Row(Modifier.defaultMinSize(MinWidth, MinHeight).padding(contentPadding))`.
- **`ButtonDefaults` là một object công khai** (:918).
  - `shape` lấy từ token `CornerFull` (:1156).
  - `ContentPadding` là 24/8 dp; bản có icon có lề đầu 16 dp (:933-948).
  - `MinHeight` là 40 dp, hoặc 36 dp khi bật cờ precision pointer (:1069-1075, PrecisionPointer.kt:32). Cờ này là công
    tắc duy nhất giống density mà Compose có.
  - Có helper theo chiều cao:
    - `contentPaddingFor(buttonHeight, hasStartIcon, hasEndIcon)`, một hàm bậc thang (:1588-1604);
    - `shapesFor`, `iconSizeFor`, `textStyleFor` (:1556-1661).

    Hàm đầu chính là `has-data-[icon=inline-start]:pl-2` của shadcn, viết thành một hàm của slot.
- **`ButtonColors` là dữ liệu cho một mục đích** (:1822-1887).
  - Nó có bốn màu: container, content, và bản disabled của mỗi màu. Màu được giải qua `containerColor(enabled)`.
  - `copy()` dùng `Color.Unspecified` với nghĩa "giữ giá trị gốc".
  - Bản mặc định được cache ngay trên `ColorScheme`, trong khoảng 50 field `default*Cached`
    (`defaultButtonColorsCached`, ColorScheme.kt:638-735).
- **Ô nhập là hai component dùng chung một cài đặt.**
  - `TextField` (TextField.kt:210) và `OutlinedTextField` (OutlinedTextField.kt:165) cùng gọi `CommonDecorationBox`
    (internal/TextFieldImpl.kt:117-136).
  - Chúng khác nhau ở hai chỗ: vị trí label (`Inside` hay `Cutout`), và slot container (gạch chân bằng `indicatorLine`
    hay viền).
  - Đổi state được animate bằng `NodeTransition`, không recompose (TextFieldImpl.kt:2125-2148).
  - `TextFieldColors` có 42 màu, giải theo `(enabled, isError, focused)` (TextFieldDefaults.kt:1804-1965).
- **Shape là hàm của kích thước.**
  - Interface là `Shape.createOutline(size, layoutDirection, density)` (ui-graphics Shape.kt:25-39).
  - `CornerBasedShape` thu nhỏ những góc lớn hơn cạnh ngắn (F/shape/CornerBasedShape.kt:43-77), cùng ý với luật góc
    chồng nhau của CSS.
  - `CircleShape` là `RoundedCornerShape(50)`, tức 50% cạnh ngắn (F/shape/RoundedCornerShape.kt:154-186).
  - M3 `Shapes` có thang từ `extraSmall` 4 dp tới `extraExtraLarge` 48 dp, cộng `full` và `none` (M/Shapes.kt:87-130,
    M/tokens/ShapeTokens.kt).
- **Expressive đổi shape khi nhấn mà không recompose.**
  - `ButtonShapes(shape, pressedShape)` morph giữa hai shape khi nút bị nhấn (Button.kt:1887-1949).
  - Shape đang animate được đọc bên trong `createOutline`, tức lúc layout và vẽ, không phải lúc compose
    (internal/AnimatedShape.kt:53-138).
- **`MotionScheme` có sáu spec** (M/MotionScheme.kt:44-133): spatial hoặc effects, mỗi loại có default, fast, slow.
  - `standard()` dùng spring spatial damping 0,9 (tokens/StandardMotionTokens.kt:20-31).
  - `expressive()` dùng damping 0,8/0,6/0,8 với stiffness thấp hơn (tokens/ExpressiveMotionTokens.kt:22-33).
- **`MaterialExpressiveTheme` giữ nguyên component, chỉ đổi giá trị** (M/MaterialTheme.kt:238-266): motion scheme, color
  scheme, shapes. Đây là tiền lệ gần nhất với tám style của shadcn: cùng component, khác bộ giá trị.
- **Theme M3 là một `staticCompositionLocalOf` cho cả object** (MaterialTheme.kt:285-290).
  - Tài liệu runtime viết rằng khi static local đổi, "the entirety of the content [is] recomposed".
  - Cũng tài liệu đó khuyên dùng `compositionLocalOf` cho giá trị "like a theme… [that] might change or even be animated"
    (runtime/CompositionLocal.kt:249-258).
  - M3 không theo lời khuyên này.
- **Hướng dẫn API** (G, cập nhật 2023-07-19):
  - "Prefer multiple components over style classes" (:335).
  - "Avoid grab-bag style parameters and classes, akin to `ComponentStyle` or `ComponentConfiguration`" (:337).
  - Defaults công khai, nằm trong object `ComponentDefaults` (:739, :1118). "Avoid using `null` as a marker"
    (:737-738).
  - `ComponentColors` "not introducing the overhead and complexities of the 'styles' pattern" (:1179), và phải được
    truyền tường minh (:1181).
  - CompositionLocal chỉ được đọc trong biểu thức mặc định của tham số, "so users can override them" (:446). "Avoid
    implicit inputs provided via `CompositionLocal`" (:416).
  - Giá trị đổi thường xuyên thì nhận `() -> T` (:820).

### 5.9 Lấy gì, bỏ gì

**Lấy:**
- **Foundation không có look:** crate `primitives`.
- **Component là hàm có tham số có kiểu, mặc định đọc theme trong biểu thức mặc định.** Builder của component có tham số
  có kiểu. Thiếu tham số thì dùng mặc định, đọc từ `Design` và `Theme` của cây con.
- **`ComponentDefaults` công khai và `ComponentColors` cho một mục đích:** `ButtonDefaults`, `ButtonColors`. Màu ở đây là
  dữ liệu tham chiếu vai màu của theme, giải theo state ở pha paint (§7.7).
- **Shape là hàm của kích thước:** `gui::Shape`, cộng shape theo vai trong `Design`.
- **`MotionScheme`, và Expressive là một bộ giá trị khác trên cùng component:** `Design.motion`. Tám style là tám bộ giá
  trị như vậy.
- **Content colour và text style đi qua context:** `ContentStyle` (§7.10).
- **Indication chỉ vẽ lại:** binding ở pha paint.
- **`() -> T`:** mọi setter nhận được closure (ADR-0013).
- **Padding là hàm của size và slot** (`contentPaddingFor`).

**Không lấy:**
- **Một static local cho cả theme.** Ở đây `Theme` và `Design` là signal theo nhóm (§7.12), đúng lời khuyên của chính
  tài liệu runtime.
- **Hình học là hằng trong token biên dịch.** Ở đây nó là dữ liệu của `Design`, vì tám style dùng chung một cài đặt
  (§4.10).
- **`Color.Unspecified` làm marker.** Rust có struct update:
  `ButtonColors { container: brand, ..ButtonDefaults::colors(d, variant) }`.
- **Một component cho mỗi variant** (`OutlinedButton`, `TextButton`…). Variant của shadcn chung mọi tham số và chung một
  cấu trúc defaults, và chỉ khác nhau ở dữ liệu màu. Một enum có kiểu, `match` đủ nhánh, giữ đúng API mà người dùng
  shadcn đã biết.

  Hướng dẫn của Compose chống object style "túi" chứa núm thị giác. Enum variant không phải thứ đó: nó là một lựa chọn
  có tên, không chứa núm nào.
- **Styles API** (thuộc tính do design system định nghĩa, lớp ưu tiên theo state).
  - Khối `pressed {}` thắng giá trị gốc bất kể thứ tự. Đó là specificity ẩn: đọc code không biết giá trị nào thắng.
  - Nó cũng là một cơ chế invalidation thứ hai, bên cạnh binding theo pha.
  - Chính Compose ghi API này là "Transitionary… will be removed before the Styles API is stable" (F/style/Style.kt:46).

## 6. Các toolkit khác

Commit đã đọc:
- qtdeclarative `639a53a`, gtk `d3d719a`, slint `2dbdba1`, microsoft-ui-xaml `7cc6dcc`, dioxus-components `9a75825`;
- iced `98bd48f`, floem `1351ffb`, xilem `22f32c7`, zed `a68d67e`, makepad `506ec05`, freya `91af397`.

Đường dẫn tính từ `references/<clone>/`. Các câu "đo được" là phép ghi đè đã chạy trong harness headless của chính
toolkit đó.

### 6.1 Bảng so sánh

| Toolkit | Style đổi cấu trúc? | Part và state | Chọn style; cây con | Kiểu và lỗi | Invalidation | Ghi đè một khía cạnh: instance / cây con |
| --- | --- | --- | --- | --- | --- | --- |
| iced | Không, chỉ field vẽ | `Status` đóng cho mỗi widget → `Fn(&Theme, Status) -> Style` | kiểu theme chọn lúc compile; closure cho mỗi instance; `themer` chỉ có tác dụng lúc vẽ | mọi lỗi là lỗi compile | status đổi → redraw; style tính lại mỗi lần vẽ; đổi theme → dựng lại view, layout lại | closure gọi `button::primary` rồi sửa một field / không có |
| floem | Không thêm được con (ẩn, disable thì được) | class có kiểu, selector dạng bitflag, thứ tự ưu tiên cố định | lúc chạy; map theo class; `.theme()` hoặc `.class()` của tổ tiên | setter có kiểu; giá trị là `Rc<dyn Any>`, sai kiểu thì panic | cờ `StyleReason` lọc theo selector đã dùng; layout chỉ khi `taffy::Style` khác | hover của instance thắng cả pressed (đo được) / class của tổ tiên giữ pressed |
| masonry | Không | class chuỗi cộng 4 cờ, trong các lớp của `PropertyStack` | lúc chạy; mặc định theo kiểu widget, thay cho cả app; không có cây con | giá trị có kiểu, class không ai kiểm; prop không ai đọc thì im lặng | giải lười, chỉ giải lại khi thứ đã tra đổi; prop hộp → layout, nền → pre-paint | prop cục bộ thắng mọi state (đo được) / không làm được |
| GPUI (zed) | Không | refinement `.hover`, `.active`, `.focus`, `.group_hover("tên")` gộp theo thứ tự cố định | một theme cho cả app; không có cây con | Rust có kiểu; theme JSON bỏ qua key lạ | tính lại mỗi frame; hover → render lại view; hover có thể đổi layout | chỉ có `IconButton::hover_background` / không làm được (phải thêm variant `OutlinedCustom(Hsla)`) |
| makepad | Shader, animator, cây con trong DSL | id và đường dẫn; animator ghi biến của shader | sheet áp lúc chạy; isolate cho cây con | kiểm lúc chạy, chỉ ghi log | animation vá slot GPU, không relayout | gộp `+:` (làm tách batch) / prototype cục bộ hoặc isolate |
| Qt Quick Controls 2 | Có, trong slot delegate | slot delegate (`background`, `contentItem`, `indicator`); bool đọc trong binding JS | import lúc compile hoặc trước khi nạp QML; cố định cho cả process; cây con chỉ đổi được theme | có kiểu; cảnh báo lúc chạy | NOTIFY → binding → item bẩn | `palette`, `Material.*`, hoặc thay delegate / attached property lan tới mọi control |
| Qt StyleKit (preview 6.11) | Delegate theo part | nhóm theo kiểu và state, có cascade | đổi style và theme lúc chạy | nhóm có kiểu, giá trị là QVariant | chỉ phát giá trị đã đổi | variation có tên / variation của container |
| GTK 4 | Không | tên node, class, pseudo-class | lúc chạy, cho cả display; không có provider theo cây con | chuỗi; lỗi parse không chặn | mỗi style giữ mask 64 bit những gì nó phụ thuộc; cờ AFFECTS → resize, allocate hay draw | class cộng provider ưu tiên APPLICATION / selector con cháu hoặc `--var` |
| Slint | Có: mỗi style là một component đầy đủ | id cộng `states [ … when … ]`, state khớp đầu tiên thắng | chỉ lúc compile; lúc chạy chỉ đổi được colour scheme | lỗi compile, có gợi ý tên đúng | push-dirty, pull-evaluate; renderer vẽ từng phần | chỉ có `primary` / không làm được |
| WinUI 3 / WPF | Có, và đổi được theo từng state | part theo `x:Name`, state theo chuỗi, khoá `[Control][Part][State]` | lúc chạy; implicit style; `RequestedTheme` theo cây con | chuỗi; part thiếu trả null, state lạ trả false, thiếu resource thì crash | setter tạo lười; metadata theo thuộc tính | `Background` cục bộ chỉ hiện lúc nghỉ; `ButtonBackgroundPointerOver` trong `Resources` / `Resources` của container |
| dioxus-components | Wrapper đã chép vào app thì có, CSS thì không | `data-state`, ARIA, CSS | biến CSS lúc chạy | prop có kiểu, CSS là chuỗi | trình duyệt lo | `class`, `style` / biến CSS kế thừa |
| freya | Không | state cục bộ → field của theme | context; `use_provide_theme` | thiếu khoá chuỗi của theme thì panic | render lại; diff chọn layout hay paint | builder đổi một field / cung cấp theme |

### 6.2 Bài học cho thiết kế

- **Template theo style đang lùi.**
  - **Slint.** Mỗi style cài đầy đủ từng widget, kể cả hành vi
    (`slint/internal/compiler/widgets/fluent/button.slint:9-131`).
    - Blog của Slint: "maintaining five parallel implementations of every widget has a cost we can't ignore anymore";
      "We're unifying the visual style, not the behavior".
    - Slint đã thêm `interface Button` và một test kiểm "that all styles have the same API"
      (`internal/compiler/tests/consistent_styles.rs:4`). Ở ta không cần phép kiểm đó: mỗi component chỉ có một cài đặt.
  - **Qt Quick Controls 2.**
    - Mỗi style là một file QML cho mỗi control, đặt trên template C++. Cây item khác nhau giữa các style
      (`qtdeclarative/src/quickcontrols/basic/Button.qml:24-47` so với `material/Button.qml:50-77`).
    - Style cố định cho cả process ("styles are fixed", trang khác biệt của Qt 5), để binding biên dịch được sang C++.
    - StyleKit mới đi hướng ngược lại: một file QML chung cho mỗi control cộng giá trị style, đổi style và theme lúc
      chạy, chỉ phát giá trị đã đổi (`src/labs/stylekit/qqstylekitreader.cpp:483-541`).
  - **WinUI/WPF.** ControlTemplate thay cả cây, nhưng hướng dẫn của Microsoft khuyên ngược lại:
    - "Expose properties to enable common customization scenarios without retemplating";
    - tránh style và template riêng để còn nhận được giao diện mới.
- **Khoá chuỗi cho part, state và resource sinh lỗi im lặng hoặc crash.**
  - WinUI: part thiếu trả `nullptr`, state lạ trả false; thiếu resource là "one of the most common causes of app
    crashes".
  - masonry: prop không ai đọc thì "not even log a warning message".
  - GPUI: tên group sai thì không bao giờ khớp.
  - freya: thiếu khoá theme thì panic.

  Ở ta không có khoá chuỗi nào. Style là struct `Design`, token là field của `Theme`, còn state là `Interaction` và sự
  thật có kiểu của primitive.
- **Thứ tự ưu tiên giữa state và giá trị cục bộ làm người dùng bất ngờ nhiều nhất.**
  - WinUI: `Background` cục bộ chỉ hiện lúc nghỉ, vì giá trị của visual state đứng trên giá trị cục bộ.
  - floem: hover của instance thắng cả pressed (đo được).
  - GTK: hover của theme là `background-image` vẽ đè lên `background-color`, nên muốn ghi đè phải xoá nó.
  - masonry #1643: một prop hover chưa định nghĩa làm mất viền khi hover.

  Ở ta không có tầng nào. Mỗi thuộc tính có một chủ: tham số của instance, hoặc mặc định của component. Màu theo state
  là code của component, viết một lần (§7.7, §7.8).
- **Đưa mỗi thuộc tính tới đúng pha là mẫu chung.**
  - GTK diff style cũ với style mới ra cờ AFFECTS, rồi chọn `queue_resize`, `queue_allocate` hay `queue_draw`
    (`gtk/gtk/gtkwidget.c:5379-5418`).
  - masonry: CornerRadius và Padding đòi layout; Background và BorderColor chỉ đòi pre-paint
    (`xilem/masonry_core/src/properties/mod.rs:33-42`).
  - floem chỉ layout lại khi `taffy::Style` suy ra bị khác (`floem/src/style/cx.rs:574-587`).
  - makepad vá slot GPU cho animation, không relayout (`makepad/platform/src/draw_vars.rs:155-161`).

  Bảng ở §7.12 theo cùng ý đó.
- **Máy selector cần cache mới rẻ.**
  - floem dựng `MatchedPropertiesCache` theo Chromium (`floem/src/style/cache.rs:1-25`).
  - GTK có mask phụ thuộc 64 bit và chia sẻ style giữa anh em (`gtk/gtk/gtkcssnodestylecache.c:80-147`).
  - WinUI có cache cho phép duyệt theme.

  Ở ta không có selector, nên không cần các cache đó.
- **`States` của `gui` hôm nay là thiết kế của GPUI, và mang cùng loại lỗi.**
  - zed #43324: style hover thiếu lúc layout.
  - zed #46371: trạng thái hover dùng chung giữa các pha.
  - zed #45437: hover bị kẹt khi chuột lướt nhanh.
  - Hover gọi `cx.notify(view)` và render lại cả view (`zed/crates/gpui/src/elements/div.rs:2975-2997`).

  `Interaction` theo node, đọc có track, sửa đúng những chỗ đó.
- **iced gần ta nhất.**
  - Style là `Fn(&Theme, Status) -> Style` trên enum `Status` của từng widget (`iced/widget/src/button.rs:551-560`).
  - Bản 0.13 bỏ trait `StyleSheet` (bị gọi là "quite painful and confusing", có "three moving parts"), chuyển sang
    status dạng dữ liệu cộng closure (`CHANGELOG.md:413-415`).
  - Ta tránh hai chỗ yếu của iced:
    - `Status` của iced đóng và không có focus. Ở ta state là `Interaction` theo node cộng sự thật của primitive, và
      mỗi binding đọc đúng thứ nó cần.
    - Bản 0.14 bắt widget tự xin redraw, kéo theo một loạt bug quên redraw (#2675, #2845, #3009, #2930). Ở ta binding
      tự track.
- **Đổi style lúc chạy không phải trả bằng tốc độ.** Qt QQC2 và Slint chọn style lúc compile để binding thành code máy,
  và vì thế không đổi được style lúc chạy. Ở ta style là dữ liệu mà code đã biên dịch đọc vào, nên đổi style chỉ là ghi
  signal (§7.12).
- **Theme theo cây con hiếm, và đáng giữ.**
  - zed, masonry, Slint không có.
  - GTK đang bỏ stylesheet cục bộ vì chúng "do not apply to the whole subtree" (`gtk/docs/…/migrating-4to5.md:63-74`).
  - Qt cho đổi theme theo cây con qua attached property, WinUI có `RequestedTheme`, freya có `use_provide_theme`.

  Ở ta `with_theme` và `with_design` là context có kiểu.
- **dioxus-components là bản Rust gần shadcn nhất:** primitive không style theo kiểu Radix, wrapper chép vào app, biến
  CSS. Nhưng nó chạy trên CSS của trình duyệt, nên mang nguyên các giới hạn ở §3.6.

## 7. Thiết kế đích

Thiết kế này theo Compose và Flutter, không theo CSS. Phần tách khỏi component là **giá trị**: `Theme` (token) và `Design`
(style). Cách vẽ thì mỗi component tự viết, một lần, và đọc hai giá trị đó qua defaults của nó. Giống
`MaterialExpressiveTheme` (§5.8): cùng component, khác bộ giá trị.

### 7.1 Ba crate

| Tầng | Crate | Tương ứng | Giữ | Không được có |
| --- | --- | --- | --- | --- |
| Engine | `gui` | Compose runtime, ui và phần không có look của foundation; Flutter rendering và widgets | reactive, component, layout, paint, text, input, focus, overlay và layout neo, motion, a11y; setter nhận giá trị hoặc closure; `Interaction` theo node; `Shape`; decoration; màu OKLCH; `ContentStyle` | token, bậc có tên, giá trị của shadcn, mọi khái niệm "style" ngoài thuộc tính của node |
| Hành vi | `primitives` (mới) | reka-ui; foundation của Compose; raw widget của Flutter | state machine, bàn phím, roving focus, typeahead, focus scope, dismiss, presence, collection, `Model<T>`, role và state a11y, sự thật có kiểu (`open`, `checked`, `side`, `highlighted`); `side` đọc từ `Placement` của `gui` | mọi giá trị thị giác |
| Design system | `shadcn` | `bases/reka/ui`, `registry/styles`, `themes.ts`; material3 | component (cấu trúc, cách vẽ, defaults), `Theme` và các preset, `Design` và tám hằng, bộ icon (cargo feature) | màu literal trong component |

Chiều phụ thuộc: `gui` ← `primitives` ← `shadcn` ← app.

Vì sao ba crate, không phải bốn:
- **Bản nháp đầu cần crate thứ tư** (`shadcn-styles`) để compiler chặn base viết look. Ở thiết kế này không còn ranh giới
  đó: component chính là nơi viết cách vẽ, một lần, và mọi giá trị đến từ dữ liệu.
- **Ranh giới còn lại là "component không viết giá trị thị giác cố định".** Lint màu (§8) và verdict theo style giữ nó.
- **`primitives` vẫn tách.** Compiler bảo đảm hành vi không có look, và design system khác dùng lại được hành vi đó (như
  Cupertino dựng trên raw widget của Flutter).

### 7.2 `gui`: giá trị, closure, và không biết design system

| Hôm nay | Đích | Ghi chú |
| --- | --- | --- |
| `border()`, `border_0()`, `border_b()` | `border(1.0)`, `border(0.0)`, `border_bottom(1.0)` | |
| `rounded_md()`, `rounded(Radius::MD)` | `shape(Shape::Rounded(r))` hoặc `radius(r)` | bậc radius là token của theme, giải trong `shadcn` |
| `p(4.0)`, `px(2.5)`, `gap(1.5)` (nhân `UNIT`) | `padding(16.0)`, `padding_x(10.0)`, `gap(6.0)` | px logic |
| `h(9.0)`, `size(9.0)`, `w_px(200.0)` | `height(36.0)`, `size(36.0)`, `width(200.0)` | `Length` nhận cả `pct`, `auto`, `calc` |
| `text_sm()` | `text_size(14.0)` và `line_height(20.0)`, hoặc `text_style(s)` | thang chữ là token của theme |
| `font_medium()` | `font_weight(500)` | |
| `tracking_widest()`, `leading_relaxed()` | `tracking(0.1)` (em), `line_height(Relative(1.625))` | |
| `shadow_xs()` | `shadow(s)` | không cấp phát; thang bóng là token |
| `font_sans()`, `font_mono()` | `font_family(f)` | `gui` không biết "sans" của shadcn là font nào |
| `items_center()`, `flex_col()` | `align_items(Center)`, `flex_direction(Column)` | giá trị enum |
| `icon_size(4.0)` | `icon_size(16.0)` | |
| `radius_var(px)`, `Radius { times }` | bỏ | radius của theme là dữ liệu của `shadcn` |
| `.has_icon(\|s\| …)` | bỏ | component biết slot icon của nó |
| `.hover`, `.active`, `.focus`, `.focus_visible`, `.disabled_style` | bỏ | binding đọc `Interaction` |
| `UNIT`, `RADIUS` | bỏ | |

- **Mọi setter nhận giá trị hoặc closure** (ADR-0013): `.background(c)` hoặc `.background(move || …)`. Closure gắn vào
  node và vào pha của thuộc tính đó. Một thuộc tính có đúng một chủ.
- **Thêm vào `gui`:**
  - **`Interaction` theo node:** hovered, pressed, focused, focus-visible, focus-within, mỗi cờ một signal. Hit testing
    và focus ghi chúng. Đọc có track theo từng cờ, nên binding đọc `focus_visible` không chạy lại khi hover.
  - **`Shape`** (§7.11).
  - **Decoration:**
    - viền theo từng cạnh;
    - focus ring, là outline vẽ ngoài shape, có offset;
    - viền phủ lên nội dung, có blend (`after:border after:mix-blend-darken` của avatar);
    - lớp lùi vào trong shape (inset), có viền và radius riêng: panel của drawer nổi (`before:` lùi 8 px);
    - vạch ở một cạnh (edge bar): vạch nhấn 2 px của alert ở sera (`after:`);
    - `backdrop(filters)` cắt theo shape.
  - **`hit_slop(edges)`.** Vùng bấm lớn hơn hộp là hành vi, không phải lớp vẽ `after:`.
  - **`Placement { side, align, available, anchor, origin }`** của layout neo, và `Slide::FromAnchor(px)`.
  - **`Color` theo OKLCH:** thành phần, màu tương đối, `mix` trong OKLCH, cùng phép tính CSS Color 4 với Chrome.
  - **`ContentStyle`** (§7.10).
- **Đổi tên, để mỗi tên một nghĩa:**
  - `gui::Shape` hôm nay (path cùng paint) thành `gui::Figure`. `Shape` từ đây là hình của node (§7.11).
  - `gui::Interaction` hôm nay (trạng thái input của cả cửa sổ) thành `gui::InputState`.
  - `gui::Ink` của ADR-0003 (màu hoặc `currentColor` của một stop) thành `gui::StopColor`. `Ink` từ đây là màu dạng
    dữ liệu của `shadcn` (§7.7).
  - Vị trí trong anh em là `Place`, vì `gui::Position` là `position` của CSS.
- **Không thêm:** tầng style, tên part, `LookCx`, thuộc tính kế thừa, selector, class. Ngoài thuộc tính của từng node,
  `gui` không có khái niệm "style" nào.
- **Ghi thuộc tính không cấp phát.** Bóng và grid track chuyển sang mảng inline nhỏ, để binding chạy lại khi hover mà
  không tốn byte nào.

### 7.3 `Theme`: token có kiểu

```rust
// crates/shadcn/src/theme.rs
#[derive(Clone, PartialEq)]
pub struct Theme {
    pub scheme: Scheme,          // Light | Dark
    pub colors: Palettes,        // a light and a dark `Palette`: background, foreground, card, popover, primary, …, chart, sidebar
    pub radius: f32,             // upstream's `--radius`, in px; the steps sm … 4xl are radius × 0.6 … 2.6
    pub fonts: Fonts,            // sans, heading, mono
    pub type_scale: TypeScale,   // xs … 9xl: size and line height
    pub shadows: Shadows,        // 2xs … 2xl
    pub menu: Menu,              // accent: Subtle | Bold; inverted: bool (menus always dark); translucent: bool
    pub icons: IconSet,          // Lucide | Tabler | Hugeicons | Phosphor | Remix, each a cargo feature
}
impl Theme {
    pub fn new(base: BaseColor) -> Self;          // neutral, stone, zinc, mauve, olive, mist, taupe
    pub fn accent(self, accent: Accent) -> Self;  // one of 17
}
```

- **Theme là trục thứ hai, độc lập với style**, như ở upstream: app chọn style và theme riêng rẽ.
- **Thứ có tên là token:** vai màu, bậc radius, bậc chữ, bậc bóng, font. Số đếm (padding, chiều cao) không phải token
  của theme. Chúng khác nhau giữa các style, nên là giá trị của `Design` (§7.4).
- **menuAccent `bold`** là một phép biến đổi palette: `accent = primary`.
- **menuColor là hai trục độc lập,** như bốn giá trị của upstream (§3.5):
  - `inverted`: nội dung menu cung cấp cho cây con của nó theme có scheme tối (`with_theme`), ở cả hai scheme của app.
    Ở upstream, `cn-menu-target` thành class `.dark`, nên menu vẫn tối khi app tối.
  - `translucent`: nội dung menu đọc `Theme.menu`, rồi vẽ nền `popover` alpha 0,7 với `backdrop` blur và saturate. Item
    đọc nó để chọn màu focus `foreground` alpha 0,1.
- **Icon.**
  - Component gọi icon theo vai: `IconRole::ChevronDown`, `Close`, `Check`…
  - Mỗi thư viện là một cargo feature chở bảng vai → glyph của nó; mặc định là lucide. Bảng lấy từ các prop của
    `IconPlaceholder` trong base upstream.
  - Icon có hướng tự lật dưới RTL.
- **Màu tính.** Palette là OKLCH, nên:
  - `oklch(from var(--primary) l calc(c*k) h)` thành `primary.with_chroma(|c| c * k)`;
  - `color-mix(in oklch, a, b 5%)` thành `a.mix(b, 0.05)`.
- **Lyra và sera vuông ngay trong `Design` của chúng.** Trang create của upstream còn ép radius của theme về 0 khi chọn
  hai style này (`useDesignSystemProvider.ts:73-76`). Đó là logic của trang chọn, và docs app làm giống vậy; nó không
  phải một phần của style.

### 7.4 `Design`: một style là một giá trị

`Design` chứa đúng những gì tám style không đồng ý với nhau (§3.8). Mỗi field là một giá trị có kiểu, đặt tên theo nghĩa
của nó, không theo hook.

```rust
// crates/shadcn/src/design.rs
/// A style: every value the eight upstream styles disagree on, typed and named by meaning (research §3.8).
/// No field is optional. Values all eight share are written once, in the component.
#[derive(Clone, PartialEq, Debug)]
pub struct Design {
    pub name: &'static str,
    pub size: BySize<f32>,       // control heights (§3.8.3); icon-only sizes are squares of them
    pub spacing: Spacing,        // the padding ladder, independent of height (§3.8.3)
    pub corners: Corners,        // corners of roles that several components share (§3.8.2)
    pub text: TextRoles,         // text by role (§3.8.4)
    pub focus: Focus,            // (§3.8.5)
    pub elevation: Elevation,    // (§3.8.6)
    pub field: Field,            // (§3.8.7)
    pub density: Density,        // (§3.8.8)
    pub motion: Motion,          // (§3.8.11)
    pub colors: ColorUse,        // (§3.8.11)
    pub parts: Parts,            // what roles do not explain, one struct per component (§3.8.2, §3.8.9, §3.8.10)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BySize<T> { pub xs: T, pub sm: T, pub default: T, pub lg: T }
impl<T: Copy> BySize<T> { pub const fn all(v: T) -> Self { Self { xs: v, sm: v, default: v, lg: v } } }

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Spacing {
    pub control_x: BySize<f32>,     // vega 8/10/10/10; maia 10/12/12/16; sera 12/16/24/32
    pub beside_icon: BySize<f32>,   // the side next to a start or end icon: vega 6/6/8/8
    pub gap: BySize<f32>,           // 4/4/6/6; mira 4 at every size
    pub glyph: BySize<f32>,         // icon size: vega 12/16/16/16; mira 10/12/14/16
}

/// A corner as a style writes it. `gui` shrinks overlapping radii as CSS does, so a large step on a control is a
/// stadium.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Corner {
    None,
    Step(RadiusStep),            // `rounded-md`: the theme's radius × 0.8
    Plus(RadiusStep, f32),       // `rounded-[calc(var(--radius)-5px)]`
    Capped(RadiusStep, f32),     // `rounded-[min(var(--radius-md),10px)]`
    Px(f32),                     // `rounded-[4px]`, `rounded-xs`
    Full,                        // `rounded-full`
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Corners {
    pub control: BySize<Corner>,          // button, toggle, button-group text, input-group button. vega: xs
                                          // Capped(Md, 8.0), sm Capped(Md, 10.0), default and lg Step(Md);
                                          // mira: xs Step(Sm), the others Step(Md); maia Step(Xl4); lyra None
    pub control_in_group: BySize<Corner>, // inside a button group: vega Step(Md), nova Step(Lg) at every size;
                                          // mira has no group rule, so it equals `control`
    pub field: Corner,                    // luma Step(Xl3): a stadium at control heights
    pub field_multiline: Corner,          // textarea: maia Step(Xl), luma Step(Xl2)
    pub item: Corner,                     // rows of menus, select, combobox
    pub popup: Corner,                    // content of menus, select, combobox
    pub modal: Corner,                    // dialog, alert dialog, command; rhea Capped(Xl4, 24.0)
    pub indicator: Corner,                // slider, progress, scroll thumb; the switch has its own (`parts.switch`)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TextSize { Step(TextStep), Px(f32) }   // a step of the theme's type scale, or upstream's `text-[0.625rem]`

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TextRole {
    pub size: TextSize,
    pub leading: Leading,        // Step (the step's own), Relaxed, None (`/1`)
    pub weight: u16,
    pub case: Case,              // AsWritten | Upper
    pub tracking: Tracking,      // Normal | Tight | Wide | Wider | Widest
    pub font: FontRole,          // Sans | Heading | Mono, resolved by the theme's fonts
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TextRoles {
    pub control: BySize<TextRole>, // vega 12/14/14/14, weight 500; nova's sm Px(12.8); mira xs Px(10.0), others 12
                                   // relaxed; lyra 12; sera 12, weight 600, upper, widest
    pub field: TextRole,
    pub field_narrow: TextSize,    // below 768 px: 16 px, so iOS does not zoom (mira 14, lyra 12)
    pub item: TextRole,            // menu items
    pub list_item: TextRole,       // select, combobox, command, menubar items
    pub body: TextRole,
    pub description: TextRole,
    pub title: TextRole,           // card, dialog, sheet, drawer, popover
    pub title_large: TextRole,     // alert dialog, empty
    pub label: TextRole,           // form labels; sera 12, weight 600, upper, wide
    pub label_inline: TextRole,    // beside a checkbox, radio or switch: sera 14, weight 400, as written
    pub group_label: TextRole,     // menu and select group labels
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Focus {
    pub ring: f32,               // width: 3; lyra 1; mira and sera 2. The invalid ring has the same width
    pub ring_alpha: f32,         // of the `ring` colour: 0.5 or 0.3
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Level { pub ring: Tone, pub shadow: ShadowStep }   // how a surface stands off what is behind it

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Elevation {
    pub card: Level,             // vega: ring fg/10 + xs; luma: ring fg/5 (dark fg/10) + md
    pub popup: Level,            // vega: ring fg/10 + md; maia: fg/5 + 2xl
    pub submenu: Level,
    pub modal: Level,            // vega: ring only; luma: + xl
    pub sheet: ShadowStep,
    pub control: ShadowStep,     // vega: xs on fields, outline buttons, toggles, checkbox, switch, OTP; others none
}

/// Underline fields show focus on their bottom border instead of a ring, so `Focus` has no field kind of its own.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FieldKind { Outline, Filled, Underline }

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Field {
    pub kind: FieldKind,
    pub fill: Tone,                    // vega: clear, dark input/30; maia: input/30 (an outline with a fill);
                                       // luma: input/50
    pub hover_fill: Tone,              // select in dark: input/50 in five styles
    pub disabled_fill: Tone,           // nova and lyra: input/50, dark input/80
    pub padding: [f32; 2],             // x, y: vega 10 × 4; maia 12 × 4; mira 8 × 2; sera 0 × 4
    pub multiline_padding: [f32; 2],
    pub resizable: bool,               // textarea: vega, nova, lyra
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Density {
    pub card: Pad,                     // padding and gap: vega 24, nova 16, rhea 20, sera 32
    pub card_small: Pad,
    pub dialog: Pad,
    pub alert: [f32; 2],
    pub popover: Pad,
    pub hover_card: f32,
    pub sheet: f32,                    // header and footer
    pub empty: f32,
    pub field_group: f32,              // gap: vega 28, nova 20, mira 16, sera 40
    pub menu: MenuDensity,             // content padding and item padding: lyra 0 and 8 × 8
    pub table: TableDensity,           // cell padding and item padding
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Motion {
    pub duration: Duration,            // of state transitions: 150 ms; rhea 200 ms
    pub field: FieldTransition,        // AllColors (nova, maia, lyra, mira) | TextAndRing (vega, rhea)
                                       // | TextRingAndFill (luma) | TextAndBorder (sera)
    pub drawer_overlay_fades: bool,    // rhea: no
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColorUse {
    pub outline: StateTones,           // fill of outline buttons by state (§7.7): mira has no light fill
    pub default_hover: HoverOn,        // All | LinksOnly: nova and lyra hover only links (`[a]:hover:`)
    pub secondary_hover: Tone,         // rhea: secondary mixed with 5% foreground
    pub backdrop: Backdrop,            // tint and blur behind modals
    pub separator: Tone,               // border or border/50
    pub unchecked: Tone,               // checkbox and radio: an input border, or an input/90 fill
    pub switch_off: Tone,
    pub slider_track: Tone,
    pub chip: Tone,
    pub selected: Tone,                // questionnaire choice, checked field label
}

/// What roles do not explain: one struct per component, as M3's component tokens. Each holds only values that differ
/// between styles; its full list is ported from §3.8 together with the component.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Parts {
    pub accordion: AccordionParts,     // frame: Divided { trigger_y } | Boxed { corner, padding, open: Tone }
    pub alert: AlertParts,             // corner; kind: Plain | AccentBar { width }; action offset; title text
    pub attachment: Corner,
    pub badge: BadgeParts,             // corner; kind: Box | Text; height; text (mira and sera 10 px)
    pub bubble: Corner,
    pub calendar: f32,                 // padding: Reka's calendar ignores --cell-size and --cell-radius
    pub card: CardParts,               // corner; title text (vega 16); footer: Plain | Bar | Line; header and image corners
    pub chart_tooltip: Corner,
    pub check: CheckParts,             // checkbox and radio: size, checkbox corner, filled or outlined, radio ring
    pub close: CloseParts,             // close button of dialog and sheet: offsets, filled
    pub code_block: CodeBlockParts,    // corner; action offsets
    pub command: CommandParts,         // item corner, alone and in a dialog
    pub dialog: DialogParts,           // max width; footer: Plain | Bar | Line; alert dialog gap, media, max width
    pub drawer: DrawerParts,           // frame: Edge { corner } | Floating { inset, corner, padding, shadow }; handle
    pub empty: EmptyParts,             // corner; icon media; title text (nova 14)
    pub hover_card: OverlayParts,      // corner (vega lg, rhea 3xl); width (256 or 288)
    pub input_group: InputGroupParts,  // its buttons' height ladder
    pub item: Corner,                  // the `Item` component
    pub kbd: KbdParts,                 // corner, height, padding, text
    pub menu: MenuParts,               // frame: Padded | Flush; min width (128 or 192); sub-menu edge
    pub menubar: MenubarParts,         // height (mira 36); item indicator offset
    pub nav: NavParts,                 // popup corner; viewport
    pub otp: OtpParts,                 // slots: Joined | Separate { gap }
    pub popover: OverlayParts,         // corner (rhea 3xl); title text (nova 14)
    pub progress: f32,                 // height
    pub questionnaire: Corner,
    pub select: SelectParts,           // trigger padding x: AsField | Px(f32) (rhea 12); native select icon offset
    pub sidebar: SidebarParts,         // radius scale for its subtree (×1.4 or 0); ring; menu button heights and corner
    pub slider: SliderParts,           // track, thumb, thumb ring, hit area
    pub switch: SwitchParts,           // track, thumb, travel, border, corner (lyra keeps it round), at both sizes
    pub table: TableParts,             // head height (40 or 48)
    pub tabs: TabsParts,               // list height (mira 32); trigger corner
    pub toast: Corner,
    pub toggle: ToggleParts,           // padding (rhea 10 and 8)
    pub tooltip: TooltipParts,         // corner; arrow corner and offset
}
```

- **Ba loại giá trị**, đúng như §3.8.13:
  - thang và vai: `size`, `spacing`, `corners`, `text`, `focus`, `elevation`, `field`, `density`, `motion`, `colors`;
  - lựa chọn cấu trúc: enum trong `parts`, như `AccordionFrame`, `DrawerFrame`, `AlertKind`, `BadgeKind`, `MenuFrame`,
    `OtpSlots`, `Footer`;
  - giá trị riêng của component: phần còn lại của `parts`.
- **`Corners` chỉ giữ vai mà nhiều component dùng chung.**
  - Góc của card, alert, tooltip, checkbox, badge và kbd nằm trong `parts` của chính component đó.
  - Dữ liệu ở §3.8.1 cho thấy bề mặt và chip không thành vai: card đi theo dialog chứ không theo alert, và kbd không lần
    nào khớp badge.
- **`parts` có một struct cho mỗi component**, như component token của M3.
  - Thêm một component là thêm một field.
  - Mỗi giá trị chỉ có một chủ. Ví dụ, offset của action trong alert chỉ nằm ở `parts.alert`.
  - Chữ của một component lệch khỏi vai ở một style cũng nằm ở đây. Ví dụ: tiêu đề card của vega, line-height theo
    component ở lyra và mira, trigger của menubar và nav ở sera, `mt-0.5` của mô tả ở sera.
- **Kiểu của giá trị:**
  - `Tone` là màu theo scheme, còn `StateTones` là màu theo hover và expanded (§7.7).
  - `Corner` là bậc của radius trong theme, nên đổi radius của theme thì mọi vai theo.
  - `RadiusStep`, `TextStep` và `ShadowStep` là bậc của thang trong `Theme`.
  - Còn lại là px logic.
- **Không field nào là `Option`.** Mỗi hằng ghi đủ mọi giá trị. Thêm một field thì tám hằng không compile cho tới khi mỗi
  hằng có giá trị.
- **Khi giá trị theo vai thì nói rõ.** Ở rhea, padding x của select trigger là 12; ở bảy style kia nó theo field.
  `SelectParts` ghi `AsField` hay `Px(12.0)`, tức một lựa chọn có tên mà hằng nào cũng phải viết ra, không phải một
  giá trị bỏ trống.
- **Giá trị mà cả tám style giống nhau không nằm trong `Design`.** Component viết chúng một lần. Ví dụ:
  - motion của popup: fade, zoom 95, slide 2, 100 ms;
  - màu ring và màu viền của invalid. Độ dày của ring invalid thì lấy từ `focus.ring`;
  - kích thước avatar, radio tròn.

  Nếu một style hay một design của app cần khác, thêm field; không có gì phải giữ tương thích.
- **Quy mô:** khoảng 250 giá trị cho mỗi style. Gần một nửa nằm trong `parts`. Đó là phần upstream chỉnh tay theo từng
  component, và dữ liệu của §3.8 cho thấy không vai nào thay được nó.
- **Vì sao một bộ giá trị là đủ.** Bản đầu của tài liệu này bác "một bộ núm vặn" vì cho rằng giá trị được chỉnh tay theo
  từng hook. Dữ liệu ở §3.8 sửa nhận định đó:
  - phần lớn giá trị theo vai: hai ví dụ cũ, luma 3xl/4xl và rhea `min(4xl, 24px)`, hoá ra là vai field, vai control và
    vai modal;
  - phần còn lại theo component chứ không theo hook, và một component có vài giá trị chứ không vài chục rule.

### 7.5 Tám hằng

```rust
// crates/shadcn/src/design/vega.rs (excerpt)
pub const VEGA: Design = Design {
    name: "vega",
    size: BySize { xs: 24.0, sm: 32.0, default: 36.0, lg: 40.0 },
    spacing: Spacing {
        control_x: BySize { xs: 8.0, sm: 10.0, default: 10.0, lg: 10.0 },
        beside_icon: BySize { xs: 6.0, sm: 6.0, default: 8.0, lg: 8.0 },
        gap: BySize { xs: 4.0, sm: 4.0, default: 6.0, lg: 6.0 },
        glyph: BySize { xs: 12.0, sm: 16.0, default: 16.0, lg: 16.0 },
    },
    corners: Corners {
        control: BySize { xs: Capped(Md, 8.0), sm: Capped(Md, 10.0), default: Step(Md), lg: Step(Md) },
        control_in_group: BySize::all(Step(Md)),
        field: Step(Md), field_multiline: Step(Md), item: Step(Sm), popup: Step(Md), modal: Step(Xl), indicator: Full,
    },
    focus: Focus { ring: 3.0, ring_alpha: 0.5 },
    field: Field {
        kind: FieldKind::Outline,
        fill: Tone { light: Ink::Clear, dark: Ink::Alpha(Input, 0.3) },
        padding: [10.0, 4.0], multiline_padding: [10.0, 8.0], resizable: true,
        /* … */
    },
    parts: Parts {
        accordion: AccordionParts { frame: AccordionFrame::Divided { trigger_y: 16.0 } },
        badge: BadgeParts { corner: Step(Xl4), kind: BadgeKind::Box, /* … */ },
        card: CardParts { corner: Step(Xl), /* … */ },
        /* … */
    },
    /* text, elevation, density, motion, colors */
};

// crates/shadcn/src/design/sera.rs (excerpt)
pub const SERA: Design = Design {
    name: "sera",
    size: BySize { xs: 28.0, sm: 36.0, default: 40.0, lg: 44.0 },
    corners: Corners::SQUARE,      // a named concept, shared with lyra: every role square
    focus: Focus { ring: 2.0, ring_alpha: 0.3 },
    field: Field { kind: FieldKind::Underline, padding: [0.0, 4.0], multiline_padding: [0.0, 12.0], resizable: false, /* … */ },
    parts: Parts {
        alert: AlertParts { corner: Corner::None, kind: AlertKind::AccentBar { width: 2.0 }, /* … */ },
        badge: BadgeParts { kind: BadgeKind::Text, /* … */ },
        otp: OtpParts { slots: OtpSlots::Separate { gap: 4.0 } },
        close: CloseParts { dialog: 20.0, sheet: 16.0, filled: true },
        switch: SwitchParts { corner: Corner::None, /* … */ },   // lyra's switch stays round
        /* … */
    },
    /* … */
};
```

- **Mỗi style một file**, viết từ giá trị ở §3.8 và kiểm bằng verdict của chính style đó.
- **Không hằng nào xây trên hằng khác.**
  - Viết `..NOVA` trong `LYRA` sẽ khiến một lần sửa nova lặng lẽ đổi lyra. Đó đúng là lý do delta trên vega bị bác (§9).
  - Giá trị dùng chung chỉ được đặt tên khi nó là một khái niệm, như `Corners::SQUARE` của lyra và sera. Switch tròn của
    lyra không nằm trong `Corners`, nên khái niệm đó đúng ở cả hai.
  - Design riêng của một app thì được phép bắt đầu bằng `..VEGA`: theo vega khi vega đổi là lựa chọn của app.
- **Không có module `common`, không tag hook, không pin upstream.** Khi upstream đổi, verdict cho thấy pixel nào lệch,
  và người port sửa giá trị.

### 7.6 Component viết một lần

**Button.** Defaults là API công khai, như `ButtonDefaults` của Compose:

```rust
// crates/shadcn/src/button.rs: written once, for every style
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonVariant { Default, Destructive, Outline, Secondary, Ghost, Link }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonSize { Xs, Sm, Default, Lg, IconXs, IconSm, Icon, IconLg }

pub struct ButtonDefaults;
impl ButtonDefaults {
    /// Icon-only sizes are squares of the same heights (§3.8.3).
    pub fn height(d: DesignCx, s: ButtonSize) -> f32 { d.size().get(s.step()) }

    /// The side next to an icon is tighter: upstream's `has-data-[icon=inline-start]:pl-2`, as Compose's
    /// `contentPaddingFor(height, hasStartIcon, hasEndIcon)`.
    pub fn padding(d: DesignCx, s: ButtonSize, icons: Icons) -> Edges {
        if s.icon_only() { return Edges::ZERO; }
        let sp = d.spacing();
        let (x, near) = (sp.control_x.get(s.step()), sp.beside_icon.get(s.step()));
        Edges { left: if icons.start { near } else { x }, right: if icons.end { near } else { x }, ..Edges::ZERO }
    }

    /// Inside a button group, a button takes the group's corner for its size (§3.8.2).
    pub fn corner(d: DesignCx, s: ButtonSize, group: Option<GroupSlot>) -> Corner {
        let c = d.corners();
        if group.is_some() { c.control_in_group.get(s.step()) } else { c.control.get(s.step()) }
    }

    /// Colours of a variant: data, resolved by state in `ButtonColors` (§7.7).
    pub fn colors(d: DesignCx, v: ButtonVariant) -> ButtonColors { /* reads d.colors() and d.focus() */ }
}

impl IntoElement for Button {
    fn into_element(self) -> Element {
        let (d, t, i) = (design(), theme(), Interaction::new());   // Copy handles; a signal per group, and per flag
        let group = inject::<GroupSlot>();                          // a button group gives each child its slot
        let Button { variant, size, icons, corner, colors, padding, disabled, invalid, expanded, on_click, children } = self;
        let colors = move || colors.unwrap_or_else(|| ButtonDefaults::colors(d, variant));
        primitives::pressable(i, on_click, disabled)                 // role, focus, Enter and Space
            .display(InlineFlex).align_items(Center).justify_content(Center).flex_shrink(0.0)
            // layout
            .height(move || ButtonDefaults::height(d, size))
            .padding(move || padding.unwrap_or_else(|| ButtonDefaults::padding(d, size, icons)))
            .gap(move || d.spacing().gap.get(size.step()))
            .border(1.0)
            // paint
            .shape(move || {
                let s = t.shape(corner.unwrap_or_else(|| ButtonDefaults::corner(d, size, group)));
                group.map_or(s, |g| s.square(g.joined()))            // square where buttons in a group meet
            })
            .background(move || t.ink(colors().container.get(i, expanded)))
            .border_color(move || t.ink(colors().border_for(i, invalid.get())))
            .focus_ring(move || focus_ring(d, t, i, colors().ring, invalid.get()))
            // property
            .opacity(move || if disabled.get() { 0.5 } else { 1.0 })
            // what text and icons inside read (§7.10)
            .provide_content(ContentStyle {
                color: Prop::bind(move || t.ink(colors().content.get(i, expanded))),
                text: Prop::bind(move || t.text(d.text().control.get(size.step()))),
                icon_size: Prop::bind(move || d.spacing().glyph.get(size.step())),
            })
            .children(children)
    }
}
```

- **Mỗi thuộc tính một binding, ở đúng pha.**
  - Hover chỉ chạy lại các binding đọc `hovered`: nền, và màu chữ ở outline và ghost. Tất cả ở pha paint.
  - Đổi sáng/tối chạy lại những binding gọi `t.ink`.
  - Đổi style chạy lại những binding đọc `d`.

  Không binding nào render lại component.
- **`Interaction` có một signal cho mỗi cờ.** Binding đọc `focus_visible` không chạy lại khi hover.
- **Không có `ButtonState` để style đọc.** Variant, size, icon và group là biến Rust của chính component. Style không cần
  biết chúng, vì style là dữ liệu.
- **Button group là context, không phải selector.**
  - `group` là `Option<GroupSlot>`. Button đọc nó bằng `inject` lúc dựng; nó không phải prop. Button group cung cấp
    cho mỗi con hướng của group và vị trí của con đó.
  - Ở upstream đây là hai luật:
    - `in-data-[slot=button-group]:rounded-md`: trong group, size nhỏ lấy góc của control;
    - `[&>*:not(:first-child)]:rounded-l-none`: cạnh chạm nhau vuông.

    Mira không có luật thứ nhất, nên `control_in_group` của mira bằng `control`.
- **`focus_ring` dùng chung cho mọi control,** trừ sáu component mà base viết cứng ring (§3.8.5): tabs trigger, toggle,
  badge, item, viewport của scroll area, tay cầm resizable. Sáu component đó vẽ ring của base, giống nhau ở mọi style.
  Với các control còn lại:
  - khi focus-visible: ring dày `d.focus().ring`, màu do variant cho (`colors.ring`). Đó là `ring` với alpha
    `d.focus().ring_alpha`, riêng destructive là `destructive` alpha 0,2 (tối 0,4);
  - khi invalid, kể cả khi không focus: cùng độ dày, màu `destructive` alpha 0,2 (tối 0,4), giống nhau ở mọi style.
    Invalid thắng focus.

**Input: bốn cách vẽ, ba `FieldKind`, một cài đặt.**

```rust
// crates/shadcn/src/input.rs
let kind = move || d.field().kind;
field
    .height(move || d.size().default)
    .padding(move || { let [x, y] = d.field().padding; Edges::xy(x, y) })
    .border(1.0)                                             // every kind keeps 1 px on every side, so layout matches
    .border_colors(move || {
        let c = t.ink(InputDefaults::border(i, invalid.get())); // input, ring when focused, destructive when invalid
        match kind() {
            FieldKind::Filled if !i.focus_visible() && !invalid.get() => Sides::all(Color::TRANSPARENT), // clear at rest
            FieldKind::Outline | FieldKind::Filled => Sides::all(c),
            FieldKind::Underline => Sides::bottom(c),        // the other sides stay transparent
        }
    })
    .background(move || t.ink(d.field().fill))
    .shape(move || t.shape(d.corners().field))
    .focus_ring(move || match kind() {
        FieldKind::Underline => Ring::NONE,                  // sera: only the bottom border changes
        FieldKind::Outline | FieldKind::Filled => focus_ring(d, t, i, FocusDefaults::ring(d), invalid.get()),
    })
```

- Đây là `InputBorder` cộng `filled` của Flutter (§4.9), viết thành một enum mà mọi binding liên quan cùng `match`.
- **Bốn cách vẽ, ba `FieldKind`.**
  - Ở upstream, bốn cách vẽ (outline, outline có nền, filled, gạch chân) là bốn bộ rule rải trên tám hook: input,
    textarea, select, native select, chips, input group, OTP, questionnaire input.
  - Ở đây mỗi hook đó là một component đọc cùng `d.field()`. "Outline có nền" là `Outline` với `fill` khác `Clear`.
- **Một sự thật, một chủ.** Field gạch chân báo focus ở viền dưới, nên `Focus` không có field riêng cho ô nhập: cùng
  một `match` trên `kind` quyết định cả viền lẫn ring.

**Accordion: khung là dữ liệu, cây giữ nguyên.**

```rust
// crates/shadcn/src/accordion.rs
let frame = move || d.accordion().frame;                     // one signal per field of `parts`
root.border(move || if matches!(frame(), AccordionFrame::Boxed { .. }) { 1.0 } else { 0.0 })
    .clip(move || matches!(frame(), AccordionFrame::Boxed { .. }))
    .shape(move || t.shape(match frame() { AccordionFrame::Boxed { corner, .. } => corner, _ => Corner::None }));
item.border_bottom(move || if place.last() { 0.0 } else { 1.0 })   // the parent gives each item its place
    .background(move || match frame() {
        AccordionFrame::Boxed { open, .. } if is_open.get() => t.ink(open),
        _ => Color::TRANSPARENT,
    });
trigger
    .padding(move || match frame() {
        AccordionFrame::Divided { trigger_y } => Edges::y(trigger_y),
        AccordionFrame::Boxed { padding, .. } => Edges::all(padding),
    })
    .focus_ring(move || focus_ring(d, t, i, FocusDefaults::ring(d), false));   // boxed too: upstream's gap is fixed (§3.8.12)
```

- **Đổi vega ↔ maia không render lại gì.** Trong accordion, các binding đọc `frame()` chạy lại, cùng các binding đọc
  nhóm khác mà maia đổi (góc, khoảng cách, chữ). Sau đó chỉ pha layout và paint của đúng những node đó chạy.
- **Người dùng không còn phải `.border_0()` cho item cuối.** Cha cho mỗi item một `Place` (đầu, cuối, chỉ số) lúc
  render.

### 7.7 Defaults và màu theo state

```rust
/// A colour as data, resolved against the palette in scope when the binding runs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ink {
    Role(ColorRole),                  // `bg-primary`
    Alpha(ColorRole, f32),            // `bg-primary/80`
    Mix(ColorRole, ColorRole, f32),   // `color-mix(in oklch, var(--secondary), var(--foreground) 5%)`
    Content,                          // the colour the enclosing container provides (`ContentStyle`, §7.10)
    Fixed(Color),                     // an app's own colour
    Clear,                            // `bg-transparent`
}

/// One ink per scheme. Upstream's `dark:` is a style's decision, so it is data, not a branch in code.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tone { pub light: Ink, pub dark: Ink }
impl Tone { pub const fn of(ink: Ink) -> Self { Self { light: ink, dark: ink } } }

/// A tone for each combination of the two states that change a button's fill and text: what Chrome shows, as data.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StateTones { pub rest: Tone, pub hovered: Tone, pub expanded: Tone, pub expanded_hovered: Tone }
impl StateTones {
    /// Hover changes the tone; being expanded changes nothing (the default variant).
    pub const fn hover(rest: Tone, hovered: Tone) -> Self {
        Self { rest, hovered, expanded: rest, expanded_hovered: hovered }
    }
    pub fn get(&self, i: Interaction, expanded: Prop<bool>) -> Tone {
        match (expanded.get(), i.hovered()) {
            (false, false) => self.rest,
            (false, true) => self.hovered,
            (true, false) => self.expanded,
            (true, true) => self.expanded_hovered,
        }
    }
}

/// Colours of one button variant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ButtonColors {
    pub container: StateTones,        // secondary: open stays at rest, hovered or not; outline and ghost: open as hovered
    pub content: StateTones,          // outline and ghost: `Ink::Content` at rest, `foreground` when hovered or open
    pub border: Tone,
    pub border_focused: Tone,         // `ring`; destructive: destructive/40
    pub ring: Tone,                   // focus ring: `ring` at the style's alpha; destructive: destructive/20, dark /40
}
impl ButtonColors {
    /// Which state wins is written once, here.
    pub fn border_for(&self, i: Interaction, invalid: bool) -> Tone {
        if invalid { INVALID_BORDER } else if i.focus_visible() { self.border_focused } else { self.border }
    }
}

/// The same in all eight styles, so it is the component's constant, not a field of `Design`.
const INVALID_BORDER: Tone = Tone { light: Ink::Role(Destructive), dark: Ink::Alpha(Destructive, 0.5) };
```

- **Màu theo state là dữ liệu cộng một hàm giải,** như Compose (`ButtonColors.containerColor(enabled)`,
  `TextFieldColors(enabled, isError, focused)`) và Flutter (`WidgetStateProperty.resolveWith`).
  - **Nền và màu chữ là bảng `StateTones`, 2 × 2 theo hover và expanded.** Bảng ghi đúng màu mà Chrome hiện ở mỗi ô,
    nên không cần luật ưu tiên nào. Upstream đạt cùng kết quả bằng thứ tự variant của Tailwind:
    - secondary đang mở giữ màu nghỉ, kể cả khi hover;
    - outline và ghost đang mở giống khi hover;
    - default đang mở không đổi gì.
  - **Viền và ring theo một thứ tự, viết một lần trong code:** invalid thắng focus-visible.
  - **Disabled làm mờ cả control:** opacity 0,5, giống nhau ở mọi style.
- **`Ink::Content` là màu mà container bao ngoài cung cấp.**
  - Outline và ghost không đặt màu chữ lúc nghỉ, nên chữ theo container. Ví dụ, trong một alert destructive, chữ của
    chúng đỏ.
  - Ở upstream đây là màu chữ kế thừa. Ở đây đó là một phép đọc tường minh `content().color`, như `IconButton` của
    Compose lấy `LocalContentColor.current`.
- **Theme giải giá trị trong binding.** Mỗi phép đọc có track, nên binding chỉ chạy lại khi đúng nhóm đó đổi:
  - `t.ink(tone)` đọc scheme và palettes;
  - `t.shape(corner)` đọc radius;
  - `t.text(role)` đọc thang chữ và font;
  - `t.shadow(step)` đọc thang bóng.
- **App đưa màu của mình qua `Ink::Fixed`.** Lint chỉ cấm tạo `Color` trong component của shadcn, không cấm app.
- **Defaults là điểm bắt đầu.**
  - App viết `ButtonColors { container: StateTones::hover(brand, brand_hover), ..ButtonDefaults::colors(d, variant) }`
    và giữ được viền, màu chữ, focus và invalid của style.
  - Chỉ đặt `rest` thì khi hover, nút đổi sang màu hover của style.
  - Muốn đổi màu ở mọi nơi, kể cả hover, thì đổi token: `with_theme(|t| t.colors.light.primary = BRAND, children)`.

### 7.8 Ghi đè: mỗi thuộc tính một chủ

| Phạm vi | Đổi token | Đổi style | Đổi một component |
| --- | --- | --- | --- |
| App | `install(theme, design)` | như bên trái | — |
| Cây con | `with_theme(\|t\| …, children)` | `with_design(\|d\| …, children)` | qua field của `Design`: `with_design(\|d\| d.parts.accordion = AccordionFrame::Boxed { … }, children)` |
| Instance | — | — | tham số có kiểu: `.variant`, `.size`, `.corner`, `.colors`, `.padding`; setter thô (`.shape(..)`, `.background(..)`) làm instance thành chủ của thuộc tính đó |

- **Một chủ cho mỗi thuộc tính.**
  - Component chỉ gắn binding mặc định cho những thuộc tính mà instance không đặt. Instance đặt thuộc tính nào (bằng giá
    trị hay closure) thì binding mặc định của thuộc tính đó không được gắn.
  - Đây là `widget ?? theme ?? defaults` của Flutter ở mức thuộc tính. Nó được quyết định một lần lúc dựng, không phải
    một phép gộp tầng lúc chạy.
- **Tham số có kiểu giữ hành vi theo state; setter thô thì không.**
  - `.colors(ButtonColors { container: StateTones::hover(brand, brand_hover), ..ButtonDefaults::colors(d, variant) })`
    giữ viền, màu chữ, focus và invalid của style.
  - `.background(BRAND)` làm instance thành chủ của nền. Khi đó hover không đổi nền nữa, trừ khi app tự viết một
    closure đọc `Interaction`. Đó là lựa chọn của app, và code nói rõ điều đó.
- **Cây con theo kịp thay đổi ở trên.**
  - `with_design(|d| d.field.kind = FieldKind::Underline, children)` cho cây con một `Design` là computed của `Design`
    bên ngoài cộng phép sửa.
  - Đổi style của app thì cây con nhận style mới mà vẫn giữ phép sửa của nó.
  - `MaterialTheme(...)` lồng nhau của Compose làm việc này bằng recompose. Ở đây nó là một computed.
- **Không có tầng, không có thứ tự ưu tiên giữa các state.** Màu theo state là code của component đọc dữ liệu (§7.7),
  viết một lần cho cả tám style.

### 7.9 Từ selector CSS sang sự thật có kiểu

| Cơ chế CSS hay Tailwind | Số lần trong 8 style | Thành gì |
| --- | --- | --- |
| Hook `cn-*`, `@apply`, twMerge khi cài | 435 hook, khoảng 400 rule mỗi style | không có gì tương ứng: component viết cách vẽ một lần, giá trị đến từ `Design` |
| `data-[variant=…]`, `data-[size=…]`, hook `-variant-`, `-size-` | 180, 150; 44 và 24 hook | enum là tham số của component; `match` đủ nhánh |
| custom variant `data-open`, `data-closed`, `data-checked`… | 1.200 | sự thật có kiểu của primitive: `open: Prop<bool>`, `checked: CheckedState`. Chỉ có một thư viện primitive nên không có hai phương ngữ |
| `aria-*` | 514 | tham số và sự thật có kiểu: `invalid`, `expanded`, `selected` |
| `hover:`, `focus:`, `focus-visible:`, `active:`, `disabled:`, `focus-within:` | 456, 412, 456, 43, 28, 55 | `Interaction` của node, `disabled` |
| `group-*`, tên group `/name` | 878, 1.009 | trong một component: một biến Rust trong cùng closure (`trigger.interaction()`); khác component: context |
| `has-*`, `group-has-*` | 784, 163 | slot có kiểu, đã biết lúc render: `icon_end.is_some()`, `has_footer` |
| `[&_x]` và `[&>x]`, `*:`, `**:` | 666, 504, 263 | `ContentStyle` cho chữ và icon (§7.10); part khác là hàm builder của chính nó |
| `peer-*` | 140 | context chung: `FieldCx { disabled, invalid, required }` |
| `in-*` | 88 | context do container cung cấp cho slot: `Surface::Tooltip`, `GroupSlot` |
| `not-*` | 55 | `!` |
| `dark:` | 548 | `Tone { light, dark }` trong dữ liệu (§7.7) |
| `sm:`, `md:` | 198 | `viewport().width_at_least(640.0)`, một computed có cắt: chỉ chạy lại khi vượt ngưỡng |
| `before:`, `after:` | 174, 119 | decoration của node, hoặc node thật của component |
| `last:`, `first:` | 56, 31 | `Place` do cha tính lúc render, như `divideTiles` và ToggleButtons của Flutter |
| `supports-backdrop-filter:` | 32 | luôn đúng: GPU luôn có |
| `!` (important) | 161 | không cần: mỗi thuộc tính có một chủ |
| giá trị tuỳ ý `-[…]`, thuộc tính tuỳ ý `[p:v]` | 454, 28 | giá trị Rust (`18.4`, `Corner::Capped(Xl4, 24.0)`), `calc()` của layout |
| `--reka-*` (chiều cao còn trống, bề rộng trigger, gốc transform) | — | giá trị placement có kiểu từ layout neo |
| `--radius`, `@theme inline`, `--spacing(n)` | — | `Theme` có kiểu; khoảng cách là số px trong `Design` |
| `IconPlaceholder` và transform khi cài | — | `IconRole`; `Theme.icons` chọn glyph |
| `cn-menu-target` → `.dark`, `cn-menu-translucent` | 8, 8 | nội dung menu đọc `Theme.menu` (§7.3) |
| `cn-rtl-flip`, class vật lý → class logic | 11 | `Direction` (ADR-0004); icon có hướng tự lật |
| `cn-font-heading` | 10 | vai chữ tiêu đề trong `Design` chọn font heading |

- **Slot chạy trong render của component nhận nó** (ADR-0013), nên context mà container cung cấp tới được nội dung app
  đưa vào. Vì vậy `in-*` và `peer-*` thành context.
- **Popup render trong context gốc.** Nội dung của overlay được dựng trong scope của trigger, nên vẫn thấy `Theme`,
  `Design` và `Surface` của trigger, như `OverlayPortal` của Flutter.
- **Chỗ CSS soi con thì giữ giá trị, không giữ phép soi.** Ví dụ `[&_p:not(:last-child)]:mb-4` của alert description
  thực chất là khoảng cách giữa các đoạn và không có khoảng sau đoạn cuối, tức `gap(16.0)` của container.

### 7.10 Nội dung: context tường minh, không kế thừa

```rust
// gui
#[derive(Clone, Copy)]
pub struct ContentStyle {
    pub color: Prop<Color>,          // text and icon colour (CSS's currentColor)
    pub text: Prop<TextStyle>,       // family, size, line height, weight, tracking, case
    pub icon_size: Prop<f32>,
}
impl Element {
    pub fn provide_content(self, c: ContentStyle) -> Self;   // for this node's subtree
}
pub fn content() -> ContentStyle;                              // the nearest one, or the root's
```

- **Ai cung cấp:** container chứa nội dung do app đưa vào, như button, badge, item của menu, tiêu đề, mô tả. Ví dụ button
  cung cấp màu content của variant và kiểu chữ control của `Design`.
- **Ai đọc:** chỉ `text`, `icon` và ô sửa text, và chỉ cho thứ chúng không được cho tường minh. Một `div` không đọc gì.
  Màu của nó còn được `StopColor::Current` (ADR-0003) và `Ink::Content` (§7.7) đọc.
- **Đọc có track, ở đúng pha.** Màu được đọc ở paint, kiểu chữ ở text layout. Màu content đổi khi hover thì text chỉ vẽ
  lại.
- **Không gộp ngầm.** Container muốn sửa một phần thì viết tường minh: `ContentStyle { color, ..content() }`. Không có
  luật gộp từng thuộc tính như `inherit` của CSS.
- **Khác kế thừa của CSS ở ba chỗ:**
  - container cung cấp tường minh một struct có kiểu;
  - chỉ text và icon đọc;
  - ngoài ba giá trị đó, chỉ `Direction` của ADR-0004 chảy xuống cây, vì layout đọc nó ở mọi node.

  Đây là `ProvideContentColorTextStyle` của Compose, và `DefaultTextStyle` cộng `IconTheme` của Flutter (§4.5, §4.10,
  §5.1).
- **Thay cho thuộc tính chữ kế thừa.** Hôm nay `gui` kế thừa thuộc tính chữ qua mọi node, như CSS. Từ đây:
  - span trong một đoạn lấy thứ nó không đặt từ đoạn đó (ADR-0001);
  - `StopColor::Current` giải theo `ContentStyle.color` (ADR-0003);
  - `tabular_nums` là một field của `TextStyle` mà container cung cấp (ADR-0005);
  - `Direction` vẫn kế thừa qua mọi node (ADR-0004).

  ADR-0022 thay phần kế thừa của ba ADR đầu.
- **Gốc.** `install` cung cấp `ContentStyle` của body: màu `foreground` và kiểu chữ body của `Design`. Nó thay preflight
  của Tailwind.
- **Không có màu viền mặc định.** Preflight đặt `* { border-color: var(--border) }`; ở đây component nào có viền thì
  đọc màu viền từ dữ liệu của chính nó.

### 7.11 Shape trên GPU

```rust
// gui
pub enum Shape {
    Rect,
    Rounded(Radii),                                // px per corner; overlapping radii shrink as CSS Backgrounds 3 §5.5 says
    Stadium,                                       // half the short side, at any size
    Superellipse { radius: f32, smoothing: f32 },  // CSS `corner-shape: squircle`; Flutter `RoundedSuperellipseBorder`
}
pub struct Radii { pub top_left: f32, pub top_right: f32, pub bottom_right: f32, pub bottom_left: f32 }
impl Radii { pub const FULL: f32; }                // as round as the box allows, as CSS's `calc(infinity * 1px)`
impl Shape { pub fn square(self, sides: Sides<bool>) -> Shape; }   // the corners on those sides become 0

// shadcn: a style's `Corner` (§7.4) becomes a `Shape` against the theme's radius
impl ThemeCx {
    pub fn shape(self, c: Corner) -> Shape {
        let step = |s: RadiusStep| self.radius() * s.factor();     // sm 0.6, md 0.8, lg 1, xl 1.4, 2xl 1.8, 3xl 2.2, 4xl 2.6
        match c {
            Corner::None => Shape::Rect,
            Corner::Step(s) => Shape::Rounded(Radii::all(step(s))),
            Corner::Plus(s, px) => Shape::Rounded(Radii::all((step(s) + px).max(0.0))),
            Corner::Capped(s, px) => Shape::Rounded(Radii::all(step(s).min(px))),
            Corner::Px(px) => Shape::Rounded(Radii::all(px)),
            Corner::Full => Shape::Stadium,
        }
    }
}
```

- **`Corner` là dữ liệu của style** (§7.4). `t.shape(corner)` giải nó theo radius của theme, trong binding ở pha paint,
  nên đổi radius chỉ chạy lại binding shape.
- **Góc theo vị trí.** Button group làm vuông cạnh nơi hai button chạm nhau (`Shape::square`), kể cả khi shape là
  stadium. `Radii::FULL` là góc tròn hết mức mà hộp cho phép, nên luật góc chồng nhau tự tính nửa cạnh ngắn, không cần
  biết kích thước lúc dựng.
- **Một shape cho mọi thứ của node:** nền, viền, ring, bóng, clip và hit testing. Chrome cũng hit test theo
  border-radius.
- **Giá trị của upstream cho đúng pixel, nhờ luật góc chồng nhau.**
  - `rounded-4xl` trên control cao 36 px vẽ thành stadium. Lý do là 26 + 26 > 36, nên mọi góc được nhân với 36/52.
  - `gui` theo đúng luật đó.
  - Không thay `Step(Xl4)` bằng `Full`: khi radius của theme đổi, hai giá trị này khác nhau. Ví dụ ở radius 0,3rem,
    bậc 4xl là 12,5 px, không còn là stadium.
- **Chỗ có thể tốt hơn Chrome.** Mỗi chỗ chỉ được chọn sau khi so cạnh nhau với Chrome, và phải ghi vào `better.tsv`:
  - **Góc lồng đồng tâm.** Part bo góc nằm trong container có padding (tabs trigger trong tabs list, item trong menu)
    lấy radius ngoài trừ padding, thay cho một bậc chọn tay.
  - **Superellipse** cho design riêng của app. Tám style không dùng nó.
  - **Đổi shape có animate**, như morph khi nhấn của Expressive, đọc ở pha paint.
- **Vẽ bằng path trên GPU**, với coverage chính xác (ADR-0017).

### 7.12 Reactive và pha

- **`Theme` và `Design` là signal theo nhóm.**
  - `install` và `with_*` cung cấp một handle `Copy`, gồm một signal cho mỗi nhóm:
    - của theme: scheme, palettes, radius, fonts, type scale, shadows, menu, icons;
    - của design: mỗi field của §7.4 (size, spacing, corners, text, focus, elevation, field, density, motion, colors),
      và mỗi field của `parts` (`d.accordion()`, `d.drawer()`, `d.switch()`…). Sửa khung accordion của một cây con chỉ
      chạy lại binding của accordion.
  - Ghi cả giá trị (`theme().set(t)`) là ghi từng nhóm. Nhóm nào bằng giá trị cũ thì dừng ở equality cutoff (ADR-0012,
    bất biến 3).
  - Cây con có ghi đè dùng computed của nhóm bên ngoài, nên thay đổi ở trên vẫn tới được nó.
  - Đây là `InheritedModel` theo khía cạnh của Flutter (§4.9), áp cho theme.
- **Mỗi binding đi thẳng tới pha của thuộc tính nó gắn** (ADR-0013):

| Thuộc tính | Pha bẩn |
| --- | --- |
| display, kích thước, min/max, padding, margin, độ dày viền, gap, flex/grid, vị trí | layout của node; relayout boundary quyết định lan tới đâu (ADR-0014) |
| cỡ chữ, font, weight, tracking, line-height, chữ hoa | layout của text (shape lại) |
| màu chữ, gạch chân | paint; layout của text giữ nguyên |
| nền, màu viền, shape, bóng, ring, decoration, backdrop | paint (chunk của node) |
| opacity, transform | property (render thread, không ghi lại chunk) |
| transition, animation | không pha nào, cho tới khi giá trị có transition đổi |

- **Không có pha style và không có phép gộp.** Bản nháp đầu chạy look ở một pha style rồi gộp tầng và diff. Ở đây mỗi
  thuộc tính có binding riêng, và style diff của ADR-0014 chỉ cắt những giá trị không đổi.
- **Không component nào render lại** khi hover, focus, mở/đóng, đổi sáng/tối hay đổi style. Mọi khác biệt giữa tám style
  là giá trị thuộc tính trên cùng một cây (§3.4), nên đổi style chỉ chạy lại binding.
- **Lựa chọn làm đổi cây thì đọc trong render.**
  - Tám style không có lựa chọn nào như vậy.
  - Nếu một design của app có (ví dụ thêm một node trang trí), component đọc field đó trong render. Khi đổi, chỉ
    component đó render lại.
- **Chi phí.**
  - Binding là một closure nhỏ. Không có selector matching, invalidation set hay style sharing cache.
  - Đổi sáng/tối chỉ ghi nhóm `scheme`, nên chỉ binding gọi `t.ink` chạy lại. Ước tính: ≤ 0,5 ms cho 2.000 part đang
    mount, đo bằng bench.
  - List triệu dòng chỉ mount những dòng đang hiện (ADR-0014), nên số binding tỉ lệ với màn hình, không với dữ liệu.
  - Dòng đọc `selected` từ selection dạng khoảng chỉ chạy lại khi state của chính nó đổi (computed có cắt).
- **Mọi giá trị trong `Design` và `Theme` so sánh được** (`PartialEq`, không có con trỏ hàm). Vì vậy equality cutoff
  đúng ở mọi nhóm, và không có bão rebuild kiểu Flutter #89127.

### 7.13 App tuỳ biến thế nào

Thứ tự nên dùng, từ nhẹ tới nặng:
1. **Chọn style và theme:** `install(Theme::new(BaseColor::Neutral).accent(Accent::Blue), VEGA)`.
2. **Đổi token:** `theme().update(|t| t.colors.light.primary = …)`, hoặc `with_theme` cho một cây con.
3. **Đổi giá trị của style.**
   - Cho cả app, bằng struct update lúc compile:

     ```rust
     const BRAND: Design = Design {
         name: "brand",
         corners: Corners { control: BySize::all(Corner::Full), control_in_group: BySize::all(Corner::Full), ..VEGA.corners },
         ..VEGA
     };
     ```

   - Design của app được bắt đầu từ một hằng của shadcn (`..VEGA`): đi theo vega khi vega đổi là lựa chọn của app.
   - Cho một cây con: `with_design`.
4. **Đổi một instance:** tham số có kiểu (`.corner(Corner::Full)`, `.colors(..)`), hoặc setter thô cho thuộc tính mà
   instance muốn làm chủ.
5. **Component riêng của app.**
   - Một hàm dựng trên component của shadcn, như hướng dẫn của Compose khuyên ("their own semantically meaningful
     version", G:339).
   - Ví dụ: `brand_button(label)` gọi `button(label).variant(..).colors(BRAND_COLORS)`.
   - Không mở rộng enum variant của shadcn.
6. **Design system khác hẳn:** viết component mới trên `primitives`, như Cupertino dựng trên raw widget.

### 7.14 Devtools

- **`node @…` hiện thêm:**
  - mỗi thuộc tính và chủ của nó:
    - mặc định của component, kèm field `Design` hay `Theme` mà binding đã đọc, cùng giá trị;
    - hoặc instance, kèm `file:line` qua `#[track_caller]`;
  - `Interaction` của node, và `ContentStyle` mà nó đọc.
- **Lệnh mới:** `set style nova`, `set theme stone:blue`, `set design field.kind underline`. Dùng chúng để thấy rằng đổi
  style không render lại gì, và để xem binding nào chạy lại.
- **Docs và verdict** nhận `--style <tên>` và `--theme <base>:<accent>`.

### 7.15 Khối lượng và entropy

Bản nháp đầu đã đúng ở chỗ tách style khỏi base. Nó sai ở chỗ chọn **đơn vị** của style: nó lấy hook của CSS làm đơn vị,
tức một hàm look cho mỗi part và mỗi style. Thiết kế này lấy **giá trị** làm đơn vị.

| | Bản nháp đầu: look có kiểu | Thiết kế này: style là giá trị |
| --- | --- | --- |
| Một style là | khoảng 400 hàm look, một hàm cho mỗi hook | một hằng `Design`, khoảng 250 giá trị |
| Tám style | khoảng 3.200 hàm, ước tính 20 đến 30 nghìn dòng | tám hằng, mỗi hằng vài trăm dòng |
| Kiểu phải định nghĩa | khoảng 350 struct state, mỗi part một struct | khoảng 50 kiểu nhỏ của `Design`, phần lớn là struct vài field hoặc enum vài nhánh |
| Tên part | khai báo và khớp ở khoảng 11 chỗ (base, hợp đồng, tám style, devtools) | không có: part là một node của component, gắn binding tại chỗ |
| Quy ước không ai kiểm | tag hook, module `common`, pin upstream theo style | không có: compiler bắt đủ field, verdict chấm pixel |
| Crate | 4 (`gui`, `primitives`, `shadcn`, `shadcn-styles`) | 3 |
| Thứ tự ưu tiên | 7 mức: mặc định, style, theme, app, cây con, instance, state | không có tầng: mỗi thuộc tính một chủ; hai giá trị (`Theme`, `Design`) ở ba phạm vi |
| Pha thêm vào engine | pha style: chạy look, gộp tầng, diff | không: binding đi thẳng tới pha của thuộc tính |
| Thêm một style | khoảng 400 hàm | một hằng |
| Thêm một component | một struct state cộng một hàm look trong mỗi style (8 chỗ) | component cộng defaults của nó; thêm field cho `Design` chỉ ở chỗ các style khác nhau |
| Sửa theo upstream | tìm hàm look của hook đó, sửa | sửa giá trị; verdict của style đó chỉ ra chỗ lệch |
| Đổi style lúc chạy | chạy lại look ở mọi part đang mount, gộp tầng | binding nào đọc nhóm đã đổi thì chạy lại |

- **Mỗi sự thật nằm ở đúng một chỗ.**
  - Chiều cao button của vega nằm ở `VEGA.size`.
  - Màu của button theo hover và expanded nằm ở bảng `StateTones` của `ButtonColors`. Thứ tự giữa invalid và focus nằm
    ở `ButtonColors::border_for`.
  - Cấu trúc của button nằm ở `button.rs`.
  - Bản nháp đầu chia một nút bấm ra bốn chỗ: base khai báo part, hợp đồng khai báo state, style viết look, `common` giữ
    phần chung. Đây chính là cách CSS chia (markup, data attribute, stylesheet, `@apply`), chỉ là chép sang Rust.
- **Cái gì còn phức tạp, và vì sao không bỏ được.**
  - Khoảng một nửa giá trị của mỗi hằng nằm trong `parts`. Upstream chỉnh tay chúng theo từng component (góc của từng
    surface, hình học của switch, khoảng cách của close), và dữ liệu ở §3.8 cho thấy không vai nào giải thích được chúng.
    Đây là độ phức tạp vốn có của tám style, không phải do thiết kế sinh ra.
  - `Design` lớn dần khi port thêm component. Mỗi field mới phải có lý do: các style khác nhau ở đó. Nếu cả tám giống
    nhau thì giá trị nằm trong component.
  - Người port đọc giá trị từ stylesheet của upstream bằng mắt, một lần, như mọi nguồn tham khảo khác. Đọc sai thì
    verdict của style đó tụt điểm. Không có công cụ nào parse CSS.
- **Thứ không còn phải nghĩ tới:**
  - specificity, thứ tự `@layer`, `!`;
  - twMerge, tức là class nào thắng khi gộp;
  - tên part trùng nhau;
  - style quên một hook mà vẫn compile;
  - state struct lệch với state thật của primitive.

## 8. Kiểm chứng

1. **Verdict theo style.**
   - Verdict render 389 demo bằng component base Reka chưa mở rộng (`registry/bases/reka/ui`, đã có trong sparse set
     của `fetch.sh`), bên trong `.style-<tên>`, với CSS của tám style import vào `layer(base)`, như `main.css` của docs
     site. Đây là cách trang preview của upstream render. Bên đọc CSS là Chrome, không phải code của ta.
   - Việc ánh xạ sang new-york-v4 (`vite.config.ts:19-24`) bỏ đi.
   - Docs app nhận `--style` và `--theme`. Mỗi style có điểm riêng, và không style nào được thấp hơn lần chạy trước.
   - Demo nào không chạy được trên base thì ghi lại kèm lý do.
   - Chỗ ta cố ý khác Chrome vì tốt hơn (§7.11) ghi vào `verdict/better.tsv` kèm lý do.
2. **Compiler.**
   - `Design` không có field `Option`: mỗi style ghi rõ mọi giá trị. Thêm một field thì tám hằng không compile cho tới
     khi mỗi hằng có giá trị.
   - Variant và size là enum, `match` phải đủ nhánh.
3. **Lint: component không viết màu.**
   - `crates/shadcn/clippy.toml` cấm các hàm tạo `Color` qua `disallowed-methods`. Chỉ module palette của theme được
     phép, bằng một `#[allow]` có ghi lý do.
   - Clippy chỉ đọc file `clippy.toml` gần nhất, nên file này phải chép lại bốn khoá `allow-*-in-tests` của
     `clippy.toml` ở gốc.
   - Hình học không lint được, vì một số `f32` thuộc về cấu trúc (`flex_grow(1.0)`). Verdict theo style bắt phần này: một
     giá trị hình học viết cứng trong component sẽ sai ở style nào có giá trị khác.
4. **Reactive**, đếm bằng `FrameReport`:
   - hover một nút: chỉ binding paint đọc `hovered` chạy lại (nền; outline và ghost thêm màu chữ); 0 render, 0 layout;
   - đổi sáng/tối: 0 render, 0 layout; chỉ binding gọi `t.ink` (đọc nhóm `scheme`) chạy lại;
   - đổi radius của theme: chỉ binding shape chạy lại, chỉ paint bẩn;
   - đổi style: 0 render; layout chỉ ở node có giá trị hình học đổi;
   - cây con đã ghi đè (`with_design`) vẫn theo khi style của app đổi;
   - kết quả tăng dần bằng một lần dựng lạnh (C7) ở mọi bước của mọi scenario, dưới mọi style.
5. **Cấp phát:** binding chạy lại ở trạng thái ổn định cấp phát 0 byte, đếm theo thread (I1).
6. **Bench:** docs `--bench` chạy với từng style. Đổi style trên trang lớn nhất có một phép đo riêng.

Không có phép so computed style với Chrome, và không có công cụ nào đọc CSS của upstream. Thứ được chấm là pixel; cơ
chế của ta không cần giống CSS.

## 9. Đã cân nhắc và bác bỏ

| Phương án | Lý do bác bỏ |
| --- | --- |
| Engine CSS trong `gui`: selector, cascade, specificity, class dạng chuỗi lúc chạy | Thêm một cơ chế invalidation thứ hai: selector matching phải chạy lại khi cây hay state đổi, và Blink cần invalidation set cùng style sharing mới giữ được nó rẻ. Ta đã có binding theo node và pha. Lỗi chỉ lộ lúc chạy. Trái nguyên tắc |
| Bộ sinh mã CSS → Rust, lúc build hoặc chạy một lần | Code sinh ra mang hình CSS (danh sách utility có tiền tố variant), không mang quyết định có kiểu. Selector nào thành sự thật nào là phán đoán, không làm máy móc được (`last:border-b-0` là vị trí; `[&_p:not(:last-child)]:mb-4` là gap). Parser CSS Rust tốt nhất (lightningcss) là MPL-2.0, `deny.toml` không cho. Và trái quy tắc: không công cụ nào đọc CSS của upstream |
| Công cụ bám CSS của upstream: pin cho từng style, script liệt kê rule đổi giữa hai pin, so computed style với Chrome | Buộc code của ta theo hình file CSS của upstream: mỗi thay đổi ở upstream thành việc "port lại hook X". So computed style ép mô hình thuộc tính của ta giống CSS. Giá trị đến từ việc đọc nguồn, như đọc mọi reference, còn verdict chấm pixel |
| Một hàm look cho mỗi hook, trên một `State` có kiểu cho mỗi part, gộp theo tầng (bản nháp đầu của ADR-0022, 2026-11-21) | Mỗi mảnh của nó là một engineering mà CSS buộc shadcn làm. Hàm theo hook là cấu trúc file CSS. Look ghi một bó thuộc tính vào node là `@apply`. Tầng gộp theo thứ tự là cascade. Thuộc tính kế thừa trong `gui` là kế thừa của CSS. Tên part là `data-slot`. So computed style ép mô hình thuộc tính theo CSS. Khối lượng: khoảng 3.200 hàm, ước 20–30 nghìn dòng; khoảng 350 struct `State`; mỗi part được gọi tên ở khoảng 11 chỗ, một chỗ là chuỗi không ai kiểm |
| Template theo style: mỗi style viết lại cấu trúc control (Qt Quick Controls 2, Slint, WinUI/WPF) | shadcn bảo đảm tám style chung một DOM. Tám bản cấu trúc chỉ sinh lệch về hành vi và a11y. Slint đang lùi khỏi cách này ("five parallel implementations of every widget"), và StyleKit của Qt chuyển sang một file chung cho mỗi control (§6.2) |
| Object style "túi" cho mỗi component: mọi thuộc tính nullable (`ButtonStyle` của Flutter), hoặc thuộc tính do design system định nghĩa cùng lớp ưu tiên theo state (Styles API của Compose) | Hướng dẫn của Compose khuyên tránh (G:335-337). Null nghĩa là nhường, nên đọc code không biết ai thắng. Một schema chứa mọi thuộc tính cho mỗi part chính là bộ thuộc tính CSS cho mỗi part, tức `@apply` có kiểu. Lớp ưu tiên theo state là specificity ẩn |
| Style là delta trên vega | Sửa vega sẽ lặng lẽ đổi bảy style kia. Phần dùng chung là logic của component, đọc từ `Design`, không phải giá trị của vega |
| `Design` với mọi field là `Option`, mỗi style chỉ ghi phần khác | Đọc một style không thấy giá trị thật của nó, và đó lại là delta (trên mặc định nào?). Mỗi hằng ghi đủ mọi giá trị, và không hằng nào xây trên hằng khác (§7.5) |
| Thuộc tính kế thừa trong `gui`, như CSS: màu chữ, font, cỡ icon chảy qua mọi node | Mọi node đều thành nơi nhận, nên một giá trị đặt ở một chỗ đổi node ở xa mà không đọc code nào thấy. Compose và Flutter dùng context tường minh: container cung cấp, `Text` và `Icon` đọc (§7.10) |
| Tên part làm khoá style (`.part("button")`) | Đó là `data-slot` của CSS. Part ở đây là một node do hàm builder có kiểu dựng; devtools gọi nó theo đường dẫn node (`node @…`), không theo tên |
| Một signal cho mỗi token | Mỗi binding thêm một link cho mỗi token nó đọc, trong khi token trong một nhóm (palette, radius, font) thường đổi cùng nhau. Signal theo nhóm cho cùng phép cắt khi đổi sáng/tối hay radius, với ít link hơn |
| Một signal cho cả `Theme`, hoặc cho cả `Design` | Đổi sáng/tối sẽ chạy lại cả binding hình học, và sửa radius lúc chạy sẽ chạy lại mọi binding màu. Đây là cái giá M3 và `Theme.of` của Flutter đang trả (§4.9, §5.8) |
| Crate `shadcn-styles` riêng (bốn crate) | Không còn ranh giới nào cần compiler giữ: style là dữ liệu, component là code đọc dữ liệu. Phần còn lại (component không viết màu) do lint giữ |
| Hình học là hằng trong component (token của M3, `gen_defaults` của Flutter) | Chỉ đúng khi mỗi thư viện có một design language. Tám style trên một cài đặt, đổi được lúc chạy, cần hình học là dữ liệu |
| Mỗi variant là một component (`OutlinedButton`, `TextButton`) | Variant của shadcn chung mọi tham số, chỉ khác dữ liệu màu (§5.9). Enum có kiểu giữ API mà người dùng shadcn đã biết |
| Gộp `primitives` vào `shadcn` | Mất bảo đảm lúc compile rằng hành vi không có look. Design system khác không dùng lại được; Flutter tách material ra khỏi SDK cũng vì lý do này |
| Component đọc cây lúc giải (truy vấn cấu trúc kiểu `:has`) | Phá tính cục bộ của pha: con đổi thì cha phải giải lại. Slot có kiểu biết sẵn lúc render, rẻ hơn và chính xác |
| Giữ new-york-v4 làm đích | shadcn/ui gọi nó là legacy, và DOM của nó khác base. Nó không phải một style trên base |

## 10. Kế hoạch

Một mốc mới, **M10. Style là giá trị**, gắn với ADR-0022; nó nằm trong `docs/plan-2026-11-21.md`. C6 (M2) đưa mỗi
component có state thành instance (ADR-0013): state, prop `v-model`, part và context của nó. S6 chuyển phần hành vi
đó sang `primitives`; S8 viết lại phần look của mọi component một lần, đọc `Design` và `Theme`. M10 không chạy song
song với C6: S1, S2, S3 và S5 sửa các file `gui` mà C6 cũng sửa, và S1 đổi mọi call site trong `crates/shadcn`.

**Mục tiêu:** các dòng của bảng ở §1 "Mục tiêu".

| Mã | Task | Phụ thuộc | Kiểm |
| --- | --- | --- | --- |
| S1 | `gui`: setter nhận giá trị hoặc closure. Bỏ `UNIT`, `RADIUS`, `Radius { times }`, `radius_var`, `FontFamily::{Sans, Mono}`, mọi setter đặt tên theo bậc, `has_icon`. Đổi tên ở §7.2 (`Figure`, `InputState`, `StopColor`). Sửa mọi call site | — | Build; verdict không đổi |
| S2 | `gui`: `Interaction` theo node, đọc có track; bỏ refinement của `States` (`.hover`, `.active`, `.focus`…) | C4 | Test hover: chỉ binding paint của node đó chạy lại |
| S3 | `gui`: `Shape` (bo góc theo luật góc chồng nhau của CSS, stadium, superellipse), viền theo cạnh, focus ring có offset, viền phủ có blend, lớp inset, vạch cạnh, `backdrop` cắt theo shape, `hit_slop`, `Placement` của layout neo, `Slide::FromAnchor`; ghi thuộc tính không cấp phát | S2 | Fixture Chrome cho luật góc; verdict trang avatar, menu translucent; test cấp phát |
| S4 | `gui`: `Color` theo OKLCH: thành phần, màu tương đối, `mix` trong OKLCH theo CSS Color 4 | — | So với Chrome trên bảng màu mẫu |
| S5 | `gui`: `ContentStyle`, cung cấp theo cây con, chỉ text và icon đọc | S1 | Test: màu hover của container tới text của nó, chỉ paint bẩn |
| S6 | Crate `primitives`: chuyển hành vi từ `shadcn` (và `time_input` từ `gui`) theo reka-ui. Mỗi primitive đọc nguồn reka tương ứng (`references/reka-ui/packages/core/src/<Tên>`) trước khi viết | C1–C5 | Test hành vi; snapshot a11y |
| S7 | `shadcn`: `Theme` (7 base colour × 17 accent, radius, font, thang chữ, bóng, menu; thư viện icon là cargo feature), `Design` cùng các kiểu của nó (`Corner`, `Tone`, `FieldKind`…), signal theo nhóm, `install`, `with_theme`, `with_design` | S1, S4 | Test đơn vị; đổi theme: 0 render, 0 layout |
| S8 | `shadcn`: viết lại mọi component một lần, trên `primitives`, đọc `Design` và `Theme` qua defaults (gộp vào C6a–C6d, cùng cách chia nhóm); `clippy.toml` cấm hàm tạo `Color` | S2–S7 | Build; lint |
| S9 | Hằng `VEGA` | S8, S10 | Verdict vega ≥ 99/100 |
| S10 | Verdict: demo render base Reka cộng CSS style dưới `.style-<tên>`; `run.sh --style`; docs app nhận `--style` và `--theme` lúc khởi động | S7 | Báo cáo theo style |
| S11 | Bảy hằng còn lại, mỗi style một agent. Field nào một style cần mà `Design` chưa có thì thêm cho cả tám | S9, S10 | Verdict mỗi style ≥ 99/100 |
| S12 | Docs app: chọn style và theme lúc chạy | S9 | Test 0 render khi đổi |
| S13 | Devtools: binding nào của node đọc field nào của `Design` và `Theme`, cùng giá trị; `set style`, `set theme`, `set design <đường dẫn> <giá trị>` | S7, S8 | Snapshot lệnh |
| S14 | Viết lại quy tắc port trong `crates/shadcn/README.md` (thay "Classes translate one for one"). Mục style trong "Target architecture" của AGENTS.md đã thêm khi ADR-0022 được chấp nhận | S8 | Review |

Thứ tự:
1. S1, S2 và S4 song song; S3 sau S2, S5 sau S1.
2. S6 và S7 song song. Rồi S8, chia theo nhóm của C6a–C6d; S10 sau S7.
3. S9, rồi S11 song song.
4. S12–S14 làm cuối.

Không task nào bám CSS của upstream. Khi pin của verdict đổi, verdict theo style cho thấy pixel nào lệch. Người port
đọc thay đổi ở upstream như đọc mọi reference, rồi sửa giá trị trong `Design` hoặc code của component.

## 11. Nguồn

### Mã nguồn

- shadcn-vue `d1bf1b3a2d1a824b19f32d7253a28a43797b8253` (`crates/shadcn/verdict/.shadcn-vue`):
  - `apps/v4/registry/bases/reka/ui`, `apps/v4/registry/styles`, `apps/v4/registry/new-york-v4`;
  - `apps/v4/scripts/lib/{build-styles,create-style-map,expand-cn-classes}.ts`;
  - `registry/{config,themes,base-colors}.ts`;
  - `packages/cli/src/{tailwind.css,utils/transformers,utils/updaters/update-css-vars.ts}`.
- shadcn/ui `45bbf9ba23c9c0f63d38887947e5fb06f59c7da3` (`references/shadcn-ui`): `apps/v4/registry`,
  `packages/shadcn/src`, `packages/registry/src/transformers`.
- reka-ui (`references/reka-ui/packages/core/src`).
- androidx `201ce835298cc9e6612d01af3f02717376eb2d52` (`references/androidx`):
  - `compose/foundation/…/foundation/{style,shape,interaction}`, `Indication.kt`, `Clickable.kt`, `text/BasicText.kt`;
  - `compose/material3/…/material3/{Button,Surface,ColorScheme,MaterialTheme,Shapes,MotionScheme,Text,ContentColor,Ripple,ListItem,TextField,OutlinedTextField,TextFieldDefaults,InteractiveComponentSize}.kt`,
    `internal/{TextFieldImpl,AnimatedShape,ProvideContentColorTextStyle}.kt` và `tokens/`;
  - `compose/runtime/…/runtime/CompositionLocal.kt`; `compose/ui/ui-graphics/…/graphics/Shape.kt` (đọc từ git store);
  - `compose/docs/compose-component-api-guidelines.md`.
- Flutter `203c0c4` (`references/flutter`):
  - `packages/flutter/lib/src/{widgets,material,cupertino,rendering,painting}`, nhất là `material/theme_data.dart`
    (`VisualDensity`, `ThemeExtension`), `material/input_border.dart`, `material/input_decorator.dart`,
    `material/button_style_button.dart`, `material/material.dart`, `painting/*_border.dart`,
    `widgets/{inherited_model,media_query,widget_state,icon_theme,text}.dart`, `cupertino/{button,theme}.dart`;
  - `dev/tools/gen_defaults`;
  - `docs/contributing/Style-guide-for-Flutter-repo.md`.
- Toolkit khác, trong `references/`:
  - qtdeclarative `639a53af` (`src/quicktemplates`, `src/quickcontrols`, `src/labs/stylekit`);
  - gtk `d3d719a1` (`gtk/gtkcss*.c`, `gtk/gtkwidget.c`, `gtk/theme/Default`);
  - slint `2dbdba11` (`internal/compiler/widgets`, `internal/core/properties.rs`);
  - microsoft-ui-xaml `7cc6dcc1` (`controls/dev`, `docs/design-notes`);
  - iced `98bd48fe` (`widget/src`, `core/src/theme`);
  - floem `1351ffb1` (`src/style`);
  - xilem `22f32c75` (`masonry`, `masonry_core`);
  - zed `a68d67e0` (`crates/gpui`, `crates/ui`, `crates/theme`);
  - makepad `506ec051` (`widgets`, `platform`);
  - dioxus-components `9a758255` (`primitives`, `preview`);
  - freya `91af3972` (`crates/freya-components`).

### Tài liệu

- Compose:
  - developer.android.com: "Styles update" (`/develop/ui/compose/styles`), "Jetpack Compose phases", "Follow best
    practices";
  - "Custom design systems in Compose", "Anatomy of a theme in Compose", "Migrate to Indication and Ripple APIs",
    "Migrate from Material 2 to Material 3 in Compose".
- Flutter:
  - flutter.dev/go/material-button-system-updates (2020);
  - flutter.dev/go/material-theme-system-updates (2020);
  - design doc "Decoupled theming" (2025);
  - hướng dẫn chuyển sang `material_ui` và `cupertino_ui`;
  - breaking change "MaterialState → WidgetState".
- Issue và PR của Flutter: #53059, #89127, #102993, #134065, #89563, #130335, PR #141818, #154695, #154821.
- CSS Backgrounds and Borders 3, §5.5 "Overlapping Curves" (luật thu nhỏ góc); CSS Borders 4, `corner-shape`.
- Toolkit khác:
  - doc.qt.io/qt-5/qtquickcontrols2-differences.html;
  - slint.dev/blog/default-native-style-change;
  - learn.microsoft.com: "Guidelines for designing stylable controls", "XAML styles";
  - CHANGELOG của iced (0.13, 0.14);
  - issue của masonry #1009, #1643, #1669, #1691; floem #142, #230, #267, #1045; zed PR #43324, #45437, #46371.

### Liên quan trong repo

- ADR-0004 (hướng), ADR-0007 (motion), ADR-0012 (reactive), ADR-0013 (binding theo pha), ADR-0014 (style diff, layout),
  ADR-0016 (paint, effect), ADR-0017 (path trên GPU).
- `docs/research-2026-11-21.md`, `docs/plan-2026-11-21.md` (C6).
