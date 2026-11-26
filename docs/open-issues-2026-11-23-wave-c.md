# M10 wave C: việc còn mở khi đóng đợt

2026-11-23. Wave C của M10 (Perf, S6a, S6b, S10, cùng bốn nhánh làm cứng sinh ra từ review) bị **giới hạn thời gian**
theo quyết định của người dùng. Mỗi nhánh làm nốt vòng đang chạy, sau đó không còn vòng review Codex nào. Tài liệu này
ghi mọi phát hiện còn mở để sửa sau. Mỗi mục có vị trí (`file:dòng` tại thời điểm viết), tình huống hỏng và hướng sửa.

Số dòng sẽ trôi khi code đổi. Hãy tìm theo tên hàm hoặc tên test được nêu kèm.

## 1. Trạng thái khi đóng

| Nhánh | Vòng cuối | Review của vòng cuối |
| --- | --- | --- |
| Perf (S2, hover 0 byte; `Interaction` tạo một lần) | qua review | — |
| S6a engine (layer, focus scope, tap, inert; ADR-0024) | qua review vòng 3 | — |
| S6b `primitives` nền móng và nhóm 3–8 | nền móng và nhóm 7 qua review; nhóm 6 sửa xong vòng cuối nhưng không ai review; nhóm 3, 4, 5, 8 dừng trước vòng sửa cuối | xem §6 |
| S10 verdict theo style | qua review vòng 3 | — |
| Cold check / pointer (scroll, Shared init) | vòng 17 | **chưa review** |
| Purity / đăng ký (layout, owner, holder) | vòng 8 | **chưa review** |
| Text / field | vòng 18 | **chưa review** |
| GPU clip tree | vòng 2 | **chưa review** |

Vòng cuối của bốn nhánh làm cứng xanh trên toàn bộ kiểm tra của chính nó. Mỗi phần sửa có test fail khi bỏ phần sửa đó;
việc bỏ và khôi phục đều làm bằng Edit. Tuy vậy, chưa có reviewer độc lập nào đọc các vòng này. Khi mở lại, việc đầu tiên
là một vòng review Codex cho mỗi nhánh (theo skill `review`, đặt trần số vòng ngay từ đầu).

Cổng wave C (fmt, clippy `-D warnings`, test gui-tls/gui/primitives/shadcn, `GUI_COLD_CHECK=1` cho gui lib và primitives,
`cargo check -p gui --release`, docs bench 85 trang, verdict hai nhóm demo) ghi kết quả ở §8.

## 2. Pointer, cold check, scroll

1. **Ô nhập chữ không có generation cuộn.** `edit.rs:493`, `:501` (`TextField::scroll_to`, `settled`), `interact.rs:1390`.
   Lần cuộn để hiện con trỏ bị một delta compositor cũ (đang bay) ghi đè. Sửa: view của field mang generation riêng,
   mọi lần ghi cục bộ tăng nó, `apply_scroll` so với nó.
2. **Fling vẫn chạy sau `scroll_to` của chương trình.** `compositor/scroll.rs:517` (`committed`). Delta sau mang
   generation mới nên được nhận. Sửa: quyết định một lần ghi cục bộ có huỷ fling không (Chrome huỷ khi cuộn bằng chương
   trình); nếu có thì xoá gesture của instance khi generation đổi.
3. **Xoá `reach` trong `ScrollHandle::write` chưa có test riêng.** `scroll.rs:163`. Test C qua cả khi bỏ nó vì layout
   tính lại cửa sổ. Sửa: test đọc `handle.reach()` ngay sau `scroll_to` trong cùng frame.
4. **Wheel ở mép không còn hiện thanh cuộn overlay.** `interact.rs:1411`, `scroll.rs:156`. Ghi trùng offset không ghi gì
   nên `wheeled`/`wheels` không tăng. Sửa: đếm wheel tách khỏi lần ghi offset, nếu muốn giống Chrome.
5. **`keep_at` với layout cũ.** `scroll.rs:141`. Một hook gọi `keep_at(end)` với layout đã cũ thì không ghi gì. Sửa:
   chỉ cho gọi trong hook sau layout (`pub(crate)` hoặc assert phase).
6. **Early return trong `committed` chưa có test riêng.** `compositor/scroll.rs:501`. Sửa: đếm cấp phát từ lần commit
   đầu của compositor mới.
7. **Test arena chỉ assert "nhiều nhất +1 slot".** `engine.rs:1025`. Sửa: assert độ dài free list, hoặc pop slot tự do ở
   cuối trong `unmake`.
8. **So sánh lạnh của series coi `start` là nội dung.** `chunk/series.rs:285`. Series dựng lại có đổi gốc sẽ fail cold
   check. Sửa: quyết định `start` có phải nội dung không.
9. **Cold check O(dữ liệu) mỗi frame.** `chunk/mod.rs:108` (`same_values`) và so sánh bytes. Chấp nhận vì chỉ có ở debug.
   Sửa sau: so hash nội dung giữ trên snapshot.
10. **`detached(...)` thừa.** `environment.rs:9–11`, `focus.rs:330`; `engine::live` đã detach. Mỹ quan.
11. **Hook test công khai.** `compositor/mod.rs:723` (`#[doc(hidden)] pub fn recommit_scrollers`) dùng cho
    `tests/compose_alloc.rs`. Sửa: bỏ khi lib test đếm được cấp phát mà không làm hỏng snapshot test của devtools.
12. **Shared init thất bại trong cold frame.** `reactive/runtime.rs:613` (`unmake`). Node của nó nằm trong arena tới hết
    frame. Sửa: dispose tại chỗ khi undo log xử lý được dispose lúc đang tạm dừng.

## 3. Purity, owner, đăng ký

1. **16 lời gọi `gui::after_layout` trong `primitives`/`shadcn` không có owner.** combobox, command, menu, dialog,
   dropdown_menu, pin_input, splitter, tree, stepper, tags_input. Gọi method công khai từ timer hay `Scenario::act` sẽ
   panic ở debug và mất việc ở release. Sửa: giữ `gui::Owner::current()` lúc tạo, đẩy việc qua helper `later` như
   Select và Listbox.
2. **Owner là `None` khi primitive được tạo ngoài owner.** `primitives/src/select.rs:262`, `listbox.rs:1059`,
   `shadcn/src/toggle_group.rs:145`. Sửa: bắt buộc owner (tham số, hoặc assert trong `new`).
3. **`on_stage` nối từ hai owner chỉ giữ owner cuối.** `gui/src/element.rs:1339`. Sửa: giữ danh sách cặp (closure, owner).
4. **Mỗi stage motion là một hook đóng hộp riêng.** `gui/src/motion.rs:1275`. Sửa: gom stage theo owner vào một hook.
5. **`Keyed::get` chèn entry 0 holder không bao giờ bị xoá.** `gui/src/holder.rs:707`, `hold` không owner ở `:742`.
   Sửa: `get` không chèn trên map do hold quản lý, hoặc debug-panic với `hold` không owner.
6. **`KEPT_RENDERS = 2`.** `gui/src/component.rs:1684`. Hold thuộc scope render giữ key thêm một render sau người dùng
   cuối. Vì vậy ba giới hạn test được nới (tabs ≤ 5, steps ≤ 4, carousel). Sửa: nhả hold khi output của render rời màn
   hình (sau layout kế), không phải khi scope bị drop.
7. **`on_laid_out` ngoài scope không bao giờ huỷ đăng ký.** `gui/src/members.rs:212`. Sửa: debug-panic như `after_layout`.
8. **Id đăng ký dùng `wrapping_add`.** `gui/src/members.rs:216`. Sửa: `checked_add`.
9. **`Registry::walk` không dừng nếu mỗi bước đăng ký thêm entry.** `gui/src/holder.rs:554`. Sửa: dừng ở đuôi lúc bắt
   đầu walk.
10. **Key hàng ảo của tree gây churn focus handle.** `primitives/src/tree.rs:613`. Sửa: `peek` cho hàng chưa dựng, key
    riêng cho hàng chưa dựng.
11. **Kiểm "không đổi" của tabs/carousel clone mọi giá trị.** `primitives/src/tabs.rs:226`, `carousel.rs:748`. Sửa: dùng
    số pass của Members cùng cờ thay đổi do `push` đặt.
12. **Slot hết generation không được thu hồi.** `gui/src/component.rs:1252`. Chỉ xảy ra sau 2^32 lần dùng lại. Chấp nhận,
    hoặc đổi generation sang u64.
13. **Test `parts_that_come_and_go_across_walks_leave_nothing_linked` chưa được chứng minh fail khi bỏ phần sửa.** Sửa:
    doc test compile-fail cho thấy walk không giữ được qua lời gọi.

## 4. Text và field

1. **Hình học con trỏ hash cả dòng.** `text/mod.rs:838`, `:1815`. Một dòng nhiều MB bị hash và so khi trúng cache.
   Sửa: key theo revision của giá trị cùng khoảng dòng.
2. **Accessibility copy cả giá trị.** `access.rs:786`, `:982`. Mọi thay đổi node của field ngoài selection (cuộn, đổi cỡ)
   chạy `into_owned()`, và `build` clone danh sách con. Sửa: giữ `Arc<str>` của `Shown`, so theo revision.
3. **Quyết định dựng lại run so giá trị từng byte.** `access.rs:784`. Sửa: so revision.
4. **Một phím trong textarea 100k dòng dựng lại mọi run** (khi bật screen reader). `access.rs:460`. Sửa: dựng lại theo
   dòng, từ dòng bị sửa.
5. **`selection_in` layout lại mọi run.** `access.rs:1208`, `:1232`. Sửa: dùng lại run đã giữ.
6. **Mỗi phím copy cả giá trị và layout shape lại cả text.** `edit.rs:343`, `:352`. Sửa: rope cùng shaping theo đoạn.
7. **Ô mật khẩu O(n) mỗi lần di con trỏ.** `edit.rs:269`, `:292` (`masked`, `unmasked`). Sửa: cache bảng chỉ số ký tự
   theo revision.
8. **`caret` và `anchor` vẫn công khai.** `edit.rs:22-24`. Đặt lệch ranh giới ký tự thì `composed` panic. Sửa: private,
   đặt qua `place` hoặc `select`.
9. **Click trong lúc soạn IME có selection chưa có test.** `edit.rs:372-378` (`uncomposed`). Sửa: xác định Chrome làm gì
   rồi thêm test.
10. **Listbox không lọc lại trong lúc soạn IME.** `primitives/listbox.rs:1227`. Khớp `v-model` của Vue nhưng chưa so với
    reka. Sửa: test, hoặc đọc text đang soạn nếu cần.
11. **Field thứ hai cùng handle ở release vẫn render và nhận click.** `layout/mod.rs:1670-1683` (`claim_fields`). Sửa:
    render field thứ hai ở trạng thái trơ, kèm test chạy release.
12. **Mỗi `TextField` mang một node `Computed`.** Chưa đo bộ nhớ khi có hàng nghìn field. Sửa: đo; làm lazy nếu đáng.
13. **Anchor tiếp nối của grapheme RTL bị tách.** `text/access.rs:418`. Mới test RTL thuần và RTL trong LTR. Thiếu text
    dọc và grapheme khác hướng với run.
14. **`take_trace()` sau `settle()` luôn rỗng.** `app.rs`. Report của mỗi frame đã lấy trace. Lỗi này từng làm test qua
    khi phần sửa bị bỏ. Sửa: helper của Scenario cộng report các frame, hoặc `settle` giữ trace.

## 5. GPU clip tree

1. **Tổ tiên bo góc luôn được giữ trong chuỗi hiệu lực.** `crates/gui/src/clip.rs:111` (`needed`). 30 card bo góc lồng
   nhau cho mỗi pixel lá 30 phép thử phủ. Sửa: bỏ tổ tiên khi hình chữ nhật của clip nằm trong hình chữ nhật trong
   không chạm góc của nó.
2. **Clip xoay giữ hộp thẳng trục.** `gpu/clips.rs:377` (`mapped`), `:379` (`aligned`). Damage và lát cắt chunk to tới
   khoảng 2 lần. Tổ tiên xoay không bao giờ được bỏ. Sửa: giữ tứ giác xoay (hoặc hộp có hướng), kiểm chứa trong không
   gian của tổ tiên.
3. **Clip là tổ tiên của layer blur được áp bên trong layer, trước blur.** `gpu/frame.rs:2045`, `:2062`, `composite_op`
   ở `:1968`. Blur tràn qua mép clip; Chrome cắt kết quả đã blur. Sửa: tách chuỗi của chunk tại chỗ có effect. Clip dưới
   effect nằm trong layer, clip trên effect nằm ở composite của layer.
4. **Luật bỏ clip khi hit test khác renderer.** `clip.rs:188` (`needed` qua `moved_by`). Dưới scale hoặc vị trí lẻ, một
   click cách mép trùng nhau chưa tới một pixel có thể khác thứ được vẽ. Sửa: UI hit test trên chính entry `ClipTree`
   của renderer, theo id clip của scene.
5. **Clip không có clipper vẫn hit test như hình vuông.** `interact.rs:1172`. Sửa: cho qua `chain_coverage`.
6. **Danh sách `looked` dài.** `gpu/clips.rs:389`. O(lá × độ sâu). Sửa: dừng sớm khi cả chuỗi của tổ tiên vuông, thẳng
   trục và chứa clip.
7. **Key của layer đi lại chuỗi clip mỗi frame khi layer chuyển động.** `gpu/clips.rs:180` (`key_in`),
   `gpu/frame.rs:1052`. Sửa: cache key tương đối với layer theo (clip, transform của layer, gốc) qua các frame.
8. **Reverse link cũ sau khi đổi cha.** `gpu/clips.rs:274`. Vô hại nhưng tốn việc. Sửa: bỏ link khi cha đổi.
9. **Chu trình cha.** `Ancestry` trong `gpu/clips.rs`. Clip trong chu trình treo dưới gốc, mất phần cắt của tổ tiên, và
   không có báo lỗi. Sửa: debug assert khi commit scene.
10. **Chưa có test:** chuỗi clip dưới scale khác 1 với biên lẻ (hit test so với paint); glyph slug trong layer có filter
    dưới clip xoay; đường tăng dần sau khi clip đổi cha qua layer.
11. **Thumb của overlay scrollbar không đặt output clip.** Effect của thumb để `EffectNode::clip` là 0, nên layer của nó
    không có clip nào cắt composite. Có từ trước, ngoài phạm vi vòng clip tree. Sửa: paint đặt output clip của thumb là
    clip mà nhóm của nó nằm trong.
12. **`effects::subtree_region` cắt vùng theo clip layout chưa biến đổi.** Nội dung layer bị xoay hoặc scale có thể nằm
    ngoài region, nên bị mất khỏi texture. Có từ trước, ngoài phạm vi. Sửa: cắt region theo bounds của clip trong không
    gian của layer (hoặc theo `ClipTree::bounds` đã biến đổi).
13. **Chi phí đã chấp nhận (review vòng 2).** (a) Trong layer có texel riêng (`own_grid`), mỗi fragment đi chuỗi
    clip theo cha tới output clip, không theo chuỗi hiệu lực, vì chuỗi hiệu lực được quyết định trên pixel canvas. (b)
    Khi transform của layer đi qua identity (texel riêng sang texel của canvas hoặc ngược lại), layer vẽ lại một lần.
    (c) Clip đổi hình (`reshaped`) hay clip có key qua canvas đổi chỗ thì key của nó và của mọi clip trong cây con bị
    bỏ trong mỗi layer đã đọc nó (`ClipKeys::forget`, đi cây con một lần mỗi layer). (d) Transform có matrix hay cha
    đổi thì bỏ hết key của layer đã compose qua nó, không chỉ key qua nó.
14. **Những gì vòng 2 làm khác hoặc chưa làm đúng hẳn.** (a) Mục 1: không thử `under` với cha cũ và cha mới; mỗi layer
    ghi các transform nó đã compose qua (`ClipKeys::through`), reverse index `Plan::through_readers` tìm ra layer, đổi
    cha thế nào cũng bắt được. Reverse index (`clip_readers`, `through_readers`) giữ cặp cũ tới lần plan cả cây; cặp
    cũ chỉ tốn một lần tra. Revision và shape vẫn là hash 64 bit. (b) Vòng 4 đã sửa phần hit test: renderer công bố
    mỗi layer nó vẽ thành một `scene::LayerStage` (map tương đối với target cha, layer ngoài, output clip); một stage
    lấy mẫu lại khi map khác identity. `Presented` giữ hit table, property tree, scale và stage của cùng một frame,
    công bố khi frame được trình diễn (như hit-test region list theo từng CompositorFrame của viz). Click, devtools
    pick và wheel đầu gesture dùng một luật (`clip::hit_in`, `HitView`); không giới hạn độ sâu (8 cấp giữ trên stack,
    phần còn lại tràn ra `Vec`); trang cao hơn texture công bố stage của lần capture theo band. Mỗi frame trong ring
    của wheel giữ stage của chính nó cùng table và property tree. Devtools pick và gửi input qua frame đang trình
    diễn (`devtools::Showing`); lệnh cuộn node vào tầm nhìn chạy lại khi frame cho thấy nó được trình diễn
    (`Done::Again`); không còn đường route theo frame như layout. Vòng 5: snapshot mang key và owner, dispatch bỏ
    event khi instance đổi; lệnh devtools có tên từ chối khi màn hình hiện node khác; chỉ còn một luật hit
    (`chain_keeps` đã xoá; trước lần present đầu, luật đó chạy trên scene UI đã vẽ, không layer); lấy mẫu đi từ dưới
    lên, mỗi texel đo một lần; frame mới thành frame trên màn hình ở vblank nó được present (`OnScreen`), present hỏng
    không đẩy frame đang hiện ra khỏi danh sách giữ lại, stamp không còn giữ thì không route; property tree của mỗi
    frame chỉ chép các entry đã đổi. Còn lại, đều thường: không có phản hồi present theo frame từ wgpu, nên vblank
    nhắm tới thay cho nó (present GPU xong trễ bị coi là đã hiện sớm tối đa một refresh); mỗi `Presented` giữ bản
    property tree riêng (tối đa `POOL` + 1 = 8 bản, khoảng 0,88 MB mỗi bản ở 20.000 clip, ghi trong ADR-0015); cuộn của compositor viết lại mọi clip tuyệt đối dưới
    container, nên bản chép theo đó; node đã biến mất ở frame N+1 thì input theo frame N không trúng gì; fixture lớp
    layer và fixture hai list của `presented_routing.rs` vẫn tách. Vòng 6: một timeline trình diễn duy nhất
    (`OnScreen`, compositor sở hữu), chỉ thêm khi present thật sự xảy ra (draw không blit hay present hỏng không thêm
    gì), frame đã vào không bị đổi (capture của cửa sổ đi với bản sao), `at(t)` là truy vấn thuần; stamp là kiểu
    (`Shown`, `Seen`), không còn số 0; trước lần present đầu, click và wheel cùng chạy trên scene đã vẽ, cache theo
    commit; hover của con trỏ đứng yên được kiểm lại khi timeline tiến (`FromRender::Presented`), không phải lúc
    layout commit; lệnh devtools có tên cần node hiện ra đúng là node đó (hoặc con của nó, hoặc box nhận pointer cho
    nó), hạn 2 s là timer của đồng hồ UI; tra node của click và hover qua `NodeIndex`; stop của gradient dùng chung
    (`Arc<[_]>`). Còn lại, đều thường: mỗi present đánh thức UI thread một lần (kể cả frame chỉ compositor cuộn), chi
    phí năng lượng chưa đo; hover được kiểm khi frame được present, sớm hơn vblank của nó tối đa một refresh; input có
    stamp cũ hơn mọi frame đang giữ (`Shown::Gone`) không trúng gì, kể cả click; luật "node nhận pointer" có ở ba chỗ
    (hai chỗ trong `paint.rs`, một trong `devtools::takes_pointer`), nên gom thành một method của `Frame`. Vòng 7:
    timeline là nguồn duy nhất cho "người dùng thấy gì lúc t". Frame vào timeline ở vblank của nó dù có đổi pixel hay
    không: commit chỉ đổi hit vẫn hiện từ vblank kế. Chỉ present hỏng mới không vào. Trước lần present đầu, hai thread
    dựng snapshot bằng một constructor (`Presented::of`) theo commit, nên list thường cuộn được ngay wheel đầu. Hover
    hội tụ về frame đang hiện: idle so với `OnScreen::now`, timer đặt ở vblank kế (`next_shown`), và input route ở
    frame cũ thì được kiểm lại ở frame mới. Thông báo chỉ mang id và gộp lại (`tell`/`look`). Giữ lại theo thời gian:
    `RETAIN` 250 ms, tối đa `KEPT` 64 frame. Số frame sống có chặn, ghi trong ADR-0015 và có test. Input từ frame đã
    `Gone` không bao giờ là hit. Số của snapshot được gán lúc công bố, nên capture có nội dung khác thì có số mới. Mỗi
    lần hit hay hover có tính trước theo frame và loại sớm theo vùng (fork RC5). Luật "nhận pointer" gom thành
    `Frame::takes_pointer` (N1). Lệnh devtools có tên chờ khi bounds đang sống khác frame đang hiện (N4). Còn lại:
    bound bộ nhớ chưa phải chia sẻ, khoảng 60 MB ở 240 Hz với 20.000 clip; cần chia sẻ chunk của tree (persistent
    vector). UI kẹt quá 250 ms trong khi compositor vẫn present thì bỏ input vị trí đang giữ. Vòng 8:
    - Hover là hàm thuần của (vị trí con trỏ, frame đang hiện). Key của lần tính thật cuối cùng chỉ do move/leave
      hoặc `converge` đặt. Move `Gone` vẫn ghi vị trí. Frame và vblank kế được đọc trong một lần (`OnScreen::sample`).
    - Render thread chỉ đánh thức UI khi thứ dưới con trỏ đổi: không có con trỏ thì 0 lần; với scroll 1 px/frame
      trên hàng 10 px thì 12 lần trên 120 frame.
    - Điểm của event được map qua property tree của frame nó được route (`InputState::placing`).
    - Hit index là R-tree STR theo từng không gian transform, cộng một R-tree các không gian được refit mỗi frame,
      với bound `O(log N + k)` có test đếm.
    - `catch_up` chép nguyên tree khi số id viết lớn hơn số entry.
    - Test giữ lại frame đếm bằng token, không dùng `Weak`.
    - Đã đo 240 Hz: lúc timeline đang đầy, 66 frame là 66 bản chép; sau đó 0 lần cấp phát.

    Còn lại:
    - Refit không gian tốn `O(S)` mỗi frame. Cần `Presented` mang danh sách transform do compositor di chuyển thì mới
      bỏ được.
    - So sánh hover trên render thread chỉ xét box và cờ modal, không xét cả chuỗi tổ tiên.
    - Node mượn box của tổ tiên thì không undo được transform nằm giữa hai node.

    (c) Mục 5: frame vẽ lại layer từng
    cấp phát ba lần mỗi chunk khi dựng op (chép `segs`, một `Vec` op, chép vào cache); nay op của layer được ghi thẳng.
    Frame vẽ chunk dưới transform bằng path (`matrix_ops`) vẫn cấp phát theo item.
15. **Hình học của frame private theo kiểu (đã làm ở vòng 13, không hoãn nữa).** Field hình học của `FrameNode` và
    `viewport` của `Frame` là `pub(in crate::layout)`. Module khác đọc qua accessor nhận token `layout::Sight` (trait
    sealed). Mỗi module đọc tự tạo token của nó; token của phía input chỉ tạo được trong `interact::shown` (ADR-0015).
    Còn lại: token công khai `inspect::OUTSIDE` cho test và công cụ ngoài crate. Rust không chặn được nó bên trong
    crate, nên lint `tests/shown_reads.rs` cấm `interact.rs` gọi tên mọi token. `look.transform` vẫn là field công
    khai của `Look` và chỉ có lint giữ.

## 6. S6b: nhóm component dừng trước vòng sửa cuối

Khi giới hạn sử dụng cắt ngang, nhóm 3 (overlay), 4 (menu), 5 (select và list) và 8 (structure) còn 1–3 phát hiện quan
trọng mỗi nhóm, chưa sửa. Nhóm 6 (form control) đã sửa vòng cuối nhưng không ai review. Hai mục đã biết vẫn còn:

- `shadcn/src/resizable.rs:244`: `watch(new, old)` truyền min/max **cũ** cho `set_constraints` khi ràng buộc của một
  panel đang mount thay đổi.
- `primitives/src/drawer.rs:663`, `:1269`: `DrawerProvider::frontmost_height()` giữ chiều cao của drawer phía trước đã
  bị gỡ cho tới lần đo kế.

Danh sách đầy đủ của từng nhóm nằm trong báo cáo review Codex của nhóm đó và chưa được chép sang đây. Khi mở lại, chạy
một vòng review cho nhóm 3–6 và 8 trên code hiện tại thay vì dùng báo cáo cũ, vì các nhánh làm cứng đã sửa nhiều file
của chúng (owner, `after_layout`, holder).

## 7. Backlog chung chưa làm

Từ wave A, wave B và các review trước. Các mục đã xong trong wave C không ghi lại ở đây: hover 0 byte, `Interaction`
trong render, clip tree, định tuyến wheel của compositor.

**Hiệu năng và cấp phát**

- `churn click Mount` 88.288 so với budget 89.000 (dư 0,8%).
- `TextSystem::draw` dựng `placed: Vec<Item>` mới mỗi lần ghi (`text/mod.rs` ~1278), khoảng 9 MB cho đoạn 54k glyph.
  Dùng lại buffer.
- `frame_node` của layout clone `String` nội dung mỗi lần đổi paint (`layout/mod.rs` ~3282). Đổi sang `Arc<str>`.
- Enum `Body` của canvas: đóng hộp `TextBody`; `large_enum_variant` chỉ còn dư 8 byte.

**Đúng đắn**

- Panic trong closure binding bỏ qua dispose tracker (`layout/mod.rs:468`, `:576`), nên tracker vẫn đăng ký.
- `LayoutTree.scope` là code chết (`layout/mod.rs:985`, `:1032`, `:1099`).
- `value.rs:596`: kiểm chồng lấn chạy trước khi binding bị phủ được bỏ. Test của `value.rs` thiếu các ca
  restyle-trước-fixed, chồng một phần và setter của shape.
- `icons/mod.rs:201-224`: doc comment lặp 5 lần; cần `compile_error!` khi không bật feature icon nào.
- Luật tap: touch `Ended` thô tới `on_touch` **sau** mouse down và click của tap (`interact.rs:1647-1652`), trong khi
  Chrome gửi touchend trước. Thiếu test nhấn giữ không handler rồi di quá slop. Một nút chuột đang giữ trong lúc ngón tay
  tap thì mất lần nhấn.
- Test DateField/TimeField: nhấn cạnh segment của field không khả dụng phải focus segment đầu.
- Devtools: `a_session_reads_acts_and_writes_signals_by_text` (click giờ ghi cả signal `pointer`) và
  `paints_name_what_was_recorded_again` (flaky khi chạy song song). Đã thấy fail một lần trong vòng 8 của nhánh purity
  rồi qua khi chạy riêng.
- Gạch chân của `MarkerLinkButtonDemo` nằm cao hơn Chrome. Số đo bàn giao ở `text/mod.rs` (post table qua skrifa,
  Inter: position -348, thickness 140 trên upem 2048; Chrome đặt ở hàng thiết bị 46–47 khi 2x). Đọc
  `text_decoration_info.cc` của Chromium thật trước khi sửa. Kiểm xem variant link có `underline-offset-4` không.
- Decoration dùng stack cố định 3 box; box thứ 4 lồng vào thay box trong cùng.
- `text/tests.rs`: đổi 4 lời gọi `inline_boxes` sang `placed_inline_boxes`, xoá `TextSystem::inline_boxes`.
- Kiểm trên Chrome Linux/Windows xem decoration có layout theo px thiết bị không (zoom-for-DSF). Hiện mới đo trên macOS.

**Thuộc S8/S9**

- S3: trang verdict avatar và menu translucent chờ S8 (`overlay_border`, `backdrop`).
- S7: `theme()`/`Colors` còn tới S8; component đọc `theme()` trong render vẫn render lại khi đổi scheme. `FADE_REVEAL`
  và mặc định motion chuyển sang `Design`; workaround `primary_tint` của `bubble.rs`.
- S6b: `Div::tab_stop` nhận giá trị hoặc closure để `primitives::Roving` không render lại item khi focus di chuyển.
- Icon: 10 tên icon upstream thiếu đang là comment trong `better.tsv`; làm thành entry thật khi verdict chạy theo thư
  viện icon.
- S4: hue xám khác Chrome có chủ ý; ghi vào `better.tsv`.
- `MessageScrollerDemo`/Streaming xuống dòng sớm hơn Chrome (lệch độ rộng ngắt dòng). `CardDemo` 52 dưới tham chiếu vega.

**Việc tay của người dùng**

- Nghe thử VoiceOver, log trackpad thật.
- Trả lời Taffy #1418 và AccessKit #871.

## 8. Quyết định đã chấp nhận và lý do

| Quyết định | Lý do |
| --- | --- |
| Verdict vega tụt 99 → 64/100 cho tới S9 | S10 đổi tham chiếu sang vega thật của upstream (reka dưới `.style-vega`), new-york-v4 đã nghỉ theo ADR-0022. S9 viết giá trị `VEGA` và phải kéo về ≥ 99. |
| `Fonts.color` là Shared, chỉ thêm | Phát hiện review bị bác trong nhánh pointer. Lý do ghi trong báo cáo vòng đó, chưa chép sang đây; xét lại khi review lại. |
| Tổ tiên bo góc luôn giữ trong chuỗi clip | Đúng trước, tối ưu sau (§5.1). |
| Blur làm reset cuộn của field cắt ngắn | Chấp nhận trong nhánh text. Chưa so với Chrome; xét lại cùng §4.9. |
| `KEPT_RENDERS = 2` | Cách nhả hold an toàn đơn giản nhất cho tới khi nhả theo màn hình (§3.6). |
| Panic trong `run_post` không poison root | Chấp nhận trong nhánh purity. Lý do chưa chép sang đây; xét lại khi review lại. |
| Copy giá trị mỗi phím vào `Arc<str>` | Đúng và đơn giản. Rope là việc riêng (§4.6). |
| Shared init chạy tách khỏi scope cả ở đường live | Trước đây scope chạm vào đầu tiên sở hữu nó và có thể dispose nó dưới chân người khác. |
| Chặn cuối của `keep_at` không gộp vào `scroll_to` | Gộp vào thì `scroll_to(end)` ngay sau khi thêm hàng sẽ dừng ở cuối cũ. |
| Cold check so dữ liệu O(n) | Chỉ có ở debug (§2.9). |
| Bỏ input có trước một cú jump, khác mặc định của Chromium | Chromium mặc định cộng phần cuộn compositor còn nợ lên offset main thread vừa đặt. Nó chỉ bỏ phần đó khi main thread yêu cầu (`LayerTreeHost::DropActiveScrollDeltaNextCommit`, `layer_tree_host.cc:2271-2275`). Ta luôn bỏ, và xếp thứ tự bằng dấu thời gian trên một đồng hồ (`scroll::jumped_after`): input có trước cú jump thì bỏ, input có sau thì cộng lên chỗ vừa jump tới. Lý do: cú jump là ý định tuyệt đối của chương trình (`scroll_to`, đưa caret vào tầm nhìn), nên nó phải thắng input mà nó không thấy. Cộng phần cũ lên sẽ đưa nội dung tới một chỗ không ai yêu cầu, giống lỗi của GTK khi deceleration đè lên `set_value` (`gtkscrolledwindow.c:3659-3667`). Với gesture có phase thì đây là luật đúng. Với bánh xe có nấc thì chấp nhận được: nấc làm trước cú jump không có tác dụng, còn nấc kế tiếp cuộn tiếp từ chỗ vừa jump tới. Luật áp dụng như nhau ở cả hai phía của commit (wave C (a) vòng 4). Xét lại nếu một app cần cách của Chromium: thêm một kiểu `Move` cho phép cộng. |

## 9. Kết quả cổng wave C

Chạy ngày 2026-11-23 trên cây làm việc chưa commit, sau khi cả bốn nhánh đã báo xong vòng cuối:

| Kiểm | Kết quả |
| --- | --- |
| `cargo fmt --all --check` | sạch |
| `cargo clippy --workspace --all-targets -- -D warnings` | sạch |
| `cargo test -p gui-tls -p gui -p primitives -p shadcn` | qua hết; gui lib 733 qua, 2 bỏ qua |
| `GUI_COLD_CHECK=1 cargo test -p gui --lib` | 733 qua |
| `GUI_COLD_CHECK=1 cargo test -p primitives` | qua hết |
| `cargo check -p gui --release` | sạch |
| Docs bench `--frames=3` | 85 PASS, 0 FAIL (chạy trong vòng cuối của nhánh text và clip) |
| Verdict button/marker/empty/field/typography/card | 69/100, `lower.txt` rỗng (cùng nguồn) |
| Verdict alert/item/message-scroller/navigation-menu | 48/100, `lower.txt` rỗng (cùng nguồn) |

Docs bench và hai verdict được chạy trong vòng cuối của nhánh text, là nhánh báo xong sau cùng. Chúng không được chạy lại
trong lượt cổng này.
