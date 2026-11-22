// ADR-0021 / plan M7: what a screen reader on macOS reads of a `gui` window,
// checked from outside the process through the macOS accessibility API
// (AXUIElement), the same API VoiceOver reads. Nothing in the app is
// changed for it: the app runs as a user runs it, with its AccessKit adapter.
//
// Usage (macOS only; the terminal needs Privacy & Security > Accessibility):
//
//   crates/gui/tools/ax_check/run.sh            # builds, launches, checks, quits
//   swift crates/gui/tools/ax_check/ax_check.swift docs <pid>
//   swift crates/gui/tools/ax_check/ax_check.swift bigtable <pid>
//
// `docs` is `cargo run --release -p shadcn --example docs`; `bigtable` is
// `cargo run --release -p shadcn --example stress -- --window bigtable`.
//
// Each check prints PASS, FAIL or GAP. GAP is what the AccessKit macOS
// adapter (accesskit_macos 0.27) does not map at all, so no app can expose
// it through it today (ADR-0021's upstream work, plan M8 U1); it does not
// fail the run. "say:" lines are what VoiceOver would announce, derived from
// the attributes it reads (role description, title or label, value, "N of
// M"); the speech itself is not observable from here.

import ApplicationServices
import Foundation

// MARK: AX helpers

func attr(_ e: AXUIElement, _ name: String) -> AnyObject? {
    var v: AnyObject?
    return AXUIElementCopyAttributeValue(e, name as CFString, &v) == .success ? v : nil
}

func attrError(_ e: AXUIElement, _ name: String) -> AXError {
    var v: AnyObject?
    return AXUIElementCopyAttributeValue(e, name as CFString, &v)
}

func names(_ e: AXUIElement) -> [String] {
    var v: CFArray?
    guard AXUIElementCopyAttributeNames(e, &v) == .success, let a = v as? [String] else { return [] }
    return a
}

func paramNames(_ e: AXUIElement) -> [String] {
    var v: CFArray?
    guard AXUIElementCopyParameterizedAttributeNames(e, &v) == .success, let a = v as? [String] else { return [] }
    return a
}

func param(_ e: AXUIElement, _ name: String, _ p: AnyObject) -> AnyObject? {
    var v: AnyObject?
    return AXUIElementCopyParameterizedAttributeValue(e, name as CFString, p, &v) == .success ? v : nil
}

@discardableResult
func set(_ e: AXUIElement, _ name: String, _ value: AnyObject) -> AXError {
    AXUIElementSetAttributeValue(e, name as CFString, value)
}

func press(_ e: AXUIElement) -> AXError {
    AXUIElementPerformAction(e, kAXPressAction as CFString)
}

func axRange(_ loc: Int, _ len: Int) -> AXValue {
    var r = CFRange(location: loc, length: len)
    return AXValueCreate(.cfRange, &r)!
}

func range(_ v: AnyObject?) -> CFRange? {
    guard let v, CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var r = CFRange()
    return AXValueGetValue(v as! AXValue, .cfRange, &r) ? r : nil
}

func rect(_ v: AnyObject?) -> CGRect? {
    guard let v, CFGetTypeID(v) == AXValueGetTypeID() else { return nil }
    var r = CGRect.zero
    return AXValueGetValue(v as! AXValue, .cgRect, &r) ? r : nil
}

func str(_ e: AXUIElement, _ n: String) -> String { attr(e, n) as? String ?? "" }
func int(_ v: AnyObject?) -> Int? { (v as? NSNumber)?.intValue }
func children(_ e: AXUIElement) -> [AXUIElement] { attr(e, "AXChildren") as? [AXUIElement] ?? [] }
func show(_ r: CFRange?) -> String { r.map { "(\($0.location), \($0.length))" } ?? "none" }

/// Breadth-first, the menu bar left out.
func find(_ root: AXUIElement, limit: Int = 20000, _ pred: (AXUIElement) -> Bool) -> [AXUIElement] {
    var out: [AXUIElement] = []
    var queue = [root]
    var seen = 0
    while !queue.isEmpty && seen < limit {
        let e = queue.removeFirst()
        seen += 1
        if pred(e) { out.append(e) }
        for c in children(e) where str(c, "AXRole") != "AXMenuBar" { queue.append(c) }
    }
    return out
}

func wait<T>(_ seconds: Double, _ f: () -> T?) -> T? {
    let end = Date().addingTimeInterval(seconds)
    repeat {
        if let v = f() { return v }
        Thread.sleep(forTimeInterval: 0.1)
    } while Date() < end
    return nil
}

// MARK: Report

var failed = 0
var gaps = 0

func check(_ ok: Bool, _ what: String, _ detail: String = "") {
    if !ok { failed += 1 }
    print("\(ok ? "PASS" : "FAIL")  \(what)\(detail.isEmpty ? "" : ": \(detail)")")
}

func gap(_ what: String, _ detail: String = "") {
    gaps += 1
    print("GAP   \(what)\(detail.isEmpty ? "" : ": \(detail)")")
}

func info(_ s: String) { print("      \(s)") }

/// What VoiceOver announces landing on `e`, from what it reads: the label
/// (title, else description, else a titled child's), the value, the role
/// description, and the place in a set when one is exposed.
func announce(_ e: AXUIElement, place: String? = nil) -> String {
    var label = str(e, "AXTitle")
    if label.isEmpty { label = str(e, "AXDescription") }
    if label.isEmpty, let t = attr(e, "AXTitleUIElement") { label = str(t as! AXUIElement, "AXValue") }
    var value = attr(e, "AXValue").map { "\($0)" } ?? ""
    if label.isEmpty && value.isEmpty {
        // No name of its own: VoiceOver reads the text inside (a row's cells).
        let inner = find(e, limit: 200) { !CFEqual($0, e) && str($0, "AXRole") == "AXStaticText" && !str($0, "AXValue").isEmpty }
        if !inner.isEmpty { value = inner.prefix(6).map { str($0, "AXValue") }.joined(separator: " ") + " (from the text inside)" }
    }
    let roleDesc = str(e, "AXRoleDescription")
    var parts = [label, value, roleDesc].filter { !$0.isEmpty }
    if let place { parts.append(place) }
    return parts.joined(separator: ", ")
}

func say(_ e: AXUIElement, place: String? = nil) { print("say:  \"\(announce(e, place: place))\"") }

func same(_ a: AnyObject?, _ b: AXUIElement) -> Bool {
    guard let a else { return false }
    return CFEqual(a, b)
}

// MARK: Setup

guard CommandLine.arguments.count == 3, let pid = pid_t(CommandLine.arguments[2]) else {
    print("usage: ax_check.swift docs|bigtable <pid>")
    exit(2)
}
let mode = CommandLine.arguments[1]

if !AXIsProcessTrusted() {
    print("FAIL  this process is not trusted for accessibility (AXIsProcessTrusted() == false).")
    print("      Enable the terminal app (or whatever runs this) in System Settings >")
    print("      Privacy & Security > Accessibility, then run again.")
    exit(3)
}

let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 5)

// The adapter builds its tree when first asked (InitialTreeRequested): wait for it.
guard let window = wait(20, { () -> AXUIElement? in
    // The docs' sidebar, or the stress scene's table.
    let ready: (AXUIElement) -> Bool = mode == "docs"
        ? { str($0, "AXRole") == "AXButton" && str($0, "AXTitle") == "Input" }
        : { str($0, "AXRole") == "AXTable" }
    return (attr(app, "AXWindows") as? [AXUIElement])?.first(where: { !find($0, limit: 2000, ready).isEmpty })
}) else {
    let e = attrError(app, "AXWindows")
    print("FAIL  no window with an accessibility tree from pid \(pid) (AXWindows: AXError \(e.rawValue))")
    if e == .apiDisabled { print("      kAXErrorAPIDisabled: enable the terminal in System Settings > Privacy & Security > Accessibility.") }
    exit(1)
}
print("window: \(str(window, "AXTitle"))")

func navButton(_ title: String) -> AXUIElement? {
    find(window) { str($0, "AXRole") == "AXButton" && str($0, "AXTitle") == title }.first
}

func open(_ page: String) -> Bool {
    guard let b = navButton(page) else {
        check(false, "the sidebar has a \"\(page)\" button")
        return false
    }
    let e = press(b)
    check(e == .success, "AXPress on \"\(page)\" in the sidebar", "AXError \(e.rawValue)")
    Thread.sleep(forTimeInterval: 0.8)
    return e == .success
}

// MARK: Text fields

/// The attributes and parameterized attributes accesskit_macos 0.27 maps for
/// a node with text ranges (`isAccessibilitySelectorAllowed:` in its node.rs).
let textAttrs = ["AXValue", "AXNumberOfCharacters", "AXSelectedText", "AXSelectedTextRange", "AXInsertionPointLineNumber"]
let textParams = ["AXStringForRange", "AXAttributedStringForRange", "AXRangeForLine", "AXRangeForPosition",
                  "AXBoundsForRange", "AXLineForIndex", "AXRangeForIndex", "AXStyleRangeForIndex"]

func checkField(_ f: AXUIElement, sample: String, multiline: Bool) {
    let role = str(f, "AXRole")
    say(f)
    let have = Set(names(f))
    for a in textAttrs { check(have.contains(a), "\(role) exposes \(a)") }
    let pnames = Set(paramNames(f))
    info("parameterized attributes: \(paramNames(f).sorted().joined(separator: ", "))")
    for p in textParams { check(pnames.contains(p), "\(role) exposes \(p)") }

    // Focus.
    let before = attr(app, "AXFocusedUIElement")
    let fe = set(f, "AXFocused", kCFBooleanTrue)
    let focused = wait(3) { same(attr(app, "AXFocusedUIElement"), f) ? true : nil } ?? false
    check(fe == .success && focused, "AXFocused = true moves the app's AXFocusedUIElement to the \(role)",
          "set AXError \(fe.rawValue), before: \(before.map { announce($0 as! AXUIElement) } ?? "none")")

    // Value, as a screen reader's "set value" (and dictation) does.
    let ve = set(f, "AXValue", sample as CFString)
    let n = sample.utf16.count
    let got = wait(3) { str(f, "AXValue") == sample ? true : nil } ?? false
    check(ve == .success && got, "setting AXValue changes the field", "AXError \(ve.rawValue), value now \"\(str(f, "AXValue"))\"")
    check(int(attr(f, "AXNumberOfCharacters")) == n, "AXNumberOfCharacters == \(n)", "\(int(attr(f, "AXNumberOfCharacters")).map(String.init) ?? "none")")
    say(f)

    // The caret, moved as VoiceOver moves it (character by character, and to a word).
    let caret = { range(attr(f, "AXSelectedTextRange")) }
    info("caret after setting the value: \(show(caret()))")
    let se = set(f, "AXSelectedTextRange", axRange(2, 0))
    let moved = wait(3) { caret().map { $0.location == 2 && $0.length == 0 } == true ? true : nil } ?? false
    check(se == .success && moved, "AXSelectedTextRange = (2, 0) moves the caret", "AXError \(se.rawValue), now \(show(caret()))")
    _ = set(f, "AXSelectedTextRange", axRange(3, 0))
    let next = wait(3) { caret()?.location == 3 ? true : nil } ?? false
    check(next, "the caret moves one character on (VoiceOver's VO+→ in text)", show(caret()))
    let firstWord = sample.split(separator: " ").first.map(String.init) ?? sample
    let wl = firstWord.utf16.count
    _ = set(f, "AXSelectedTextRange", axRange(0, wl))
    let sel = wait(3) { caret().map { $0.location == 0 && $0.length == wl } == true ? true : nil } ?? false
    check(sel, "a selection of the first word is reported back", show(caret()))
    let selected = str(f, "AXSelectedText")
    check(selected == firstWord, "AXSelectedText is \"\(firstWord)\"", "\"\(selected)\"")
    if sel { print("say:  \"\(firstWord), selected\"") }

    // Reading by range, line and index: what VoiceOver's character, word and line commands read.
    let s5 = param(f, "AXStringForRange", axRange(0, wl)) as? String
    check(s5 == firstWord, "AXStringForRange(0, \(wl)) is \"\(firstWord)\"", s5 ?? "none")
    let lineOf3 = int(param(f, "AXLineForIndex", 3 as NSNumber))
    check(lineOf3 == 0, "AXLineForIndex(3) == 0", lineOf3.map(String.init) ?? "none")
    let line0 = range(param(f, "AXRangeForLine", 0 as NSNumber))
    let firstLine = sample.split(separator: "\n", omittingEmptySubsequences: false).first.map(String.init) ?? sample
    check(line0?.location == 0 && (line0?.length ?? 0) >= firstLine.utf16.count, "AXRangeForLine(0) covers the first line", show(line0))
    if let line0, let text = param(f, "AXStringForRange", axRange(line0.location, line0.length)) as? String {
        print("say:  \"\(text.trimmingCharacters(in: .newlines))\"   (VO+L, the line)")
    }
    let idx = range(param(f, "AXRangeForIndex", 3 as NSNumber))
    check(idx?.location == 3 && idx?.length == 1, "AXRangeForIndex(3) is one character", show(idx))
    let style = range(param(f, "AXStyleRangeForIndex", 3 as NSNumber))
    check(style.map { $0.location <= 3 && $0.location + $0.length > 3 } == true, "AXStyleRangeForIndex(3) holds index 3", show(style))
    let bounds = rect(param(f, "AXBoundsForRange", axRange(0, wl)))
    check(bounds.map { $0.width > 0 && $0.height > 0 } == true, "AXBoundsForRange(0, \(wl)) is a box on screen", bounds.map { "\($0)" } ?? "none")
    if let b = bounds {
        var p = CGPoint(x: b.midX, y: b.midY)
        let pv = AXValueCreate(.cgPoint, &p)!
        let at = range(param(f, "AXRangeForPosition", pv))
        check(at.map { $0.location >= 0 && $0.location < wl } == true, "AXRangeForPosition(centre of the first word) is inside it", show(at))
    }
    if let attributed = param(f, "AXAttributedStringForRange", axRange(0, n)) as? NSAttributedString {
        check(attributed.string == sample, "AXAttributedStringForRange(whole) is the value")
    } else {
        check(false, "AXAttributedStringForRange(whole) is the value", "none")
    }
    // Word movement: macOS has no word attribute; VoiceOver finds word
    // boundaries itself in the string it reads by range. Checked: the
    // string by range is exact at each word.
    var offset = 0
    var words = 0
    for w in sample.split(separator: " ", omittingEmptySubsequences: false) {
        let len = w.utf16.count
        if let got = param(f, "AXStringForRange", axRange(offset, len)) as? String, got == String(w) { words += 1 }
        offset += len + 1
    }
    let total = sample.split(separator: " ", omittingEmptySubsequences: false).count
    check(words == total, "every word reads back exactly by range (what VoiceOver's word commands read)", "\(words) of \(total)")

    if multiline {
        let lines = sample.split(separator: "\n", omittingEmptySubsequences: false)
        let second = lines.first!.utf16.count + 1
        _ = set(f, "AXSelectedTextRange", axRange(second + 2, 0))
        let ln = wait(3) { int(attr(f, "AXInsertionPointLineNumber")) == 1 ? 1 : nil }
        check(ln == 1, "AXInsertionPointLineNumber == 1 with the caret on the second line",
              int(attr(f, "AXInsertionPointLineNumber")).map(String.init) ?? "none")
        let l1 = range(param(f, "AXRangeForLine", 1 as NSNumber))
        check(l1?.location == second, "AXRangeForLine(1) starts at \(second)", show(l1))
        let li = int(param(f, "AXLineForIndex", (second + 2) as NSNumber))
        check(li == 1, "AXLineForIndex(\(second + 2)) == 1", li.map(String.init) ?? "none")
        if let l1, let t = param(f, "AXStringForRange", axRange(l1.location, l1.length)) as? String {
            print("say:  \"\(t.trimmingCharacters(in: .newlines))\"   (VO+L on the second line)")
        }
    }
}

func textFields() -> [AXUIElement] {
    find(window) { ["AXTextField", "AXTextArea"].contains(str($0, "AXRole")) }
}

// MARK: Lists and tables

/// What Chrome and AppKit expose for a row's place and a list's size, and
/// whether this element has it.
func placeOf(_ e: AXUIElement, in parent: AXUIElement?) -> String? {
    if let i = int(attr(e, "AXIndex")), let p = parent, let n = (attr(p, "AXRows") as? [AXUIElement])?.count {
        return "\(i + 1) of \(n)"
    }
    if let i = int(attr(e, "AXARIAPosInSet")), let n = int(attr(e, "AXARIASetSize")) { return "\(i) of \(n)" }
    return nil
}

func checkRows(container c: AXUIElement, rowRole: String, expectedCount: Int?) {
    let role = str(c, "AXRole")
    let have = Set(names(c))
    info("\(role) attributes: \(names(c).sorted().joined(separator: ", "))")
    let rows = attr(c, "AXRows") as? [AXUIElement]
    let kids = find(c, limit: 5000) { str($0, "AXRole") == rowRole }
    if let rows {
        check(!rows.isEmpty, "\(role) exposes AXRows", "\(rows.count) rows")
    } else {
        gap("\(role) has no AXRows",
            "accesskit_macos maps AXRows only for containers with selectable children (list box, grid, tree, menu); \(kids.count) \(rowRole) children are in the tree")
    }
    if have.contains("AXRowCount") {
        check(expectedCount.map { int(attr(c, "AXRowCount")) == $0 } ?? true, "\(role) exposes AXRowCount", "\(attr(c, "AXRowCount").map { "\($0)" } ?? "")")
    } else {
        gap("\(role) has no AXRowCount", "accesskit_macos does not read Node::row_count\(expectedCount.map { " (the list has \($0) rows)" } ?? "")")
    }
    guard let first = (rows ?? kids).first else {
        check(false, "the \(role) holds \(rowRole) rows")
        return
    }
    info("first row attributes: \(names(first).sorted().joined(separator: ", "))")
    let firstNames = Set(names(first))
    if firstNames.contains("AXIndex") {
        check(int(attr(first, "AXIndex")) != nil, "a row exposes AXIndex", "\(attr(first, "AXIndex").map { "\($0)" } ?? "")")
    } else {
        gap("a row has no AXIndex", "accesskit_macos does not read Node::row_index")
    }
    if !firstNames.contains("AXARIAPosInSet") {
        gap("a row has no AXARIAPosInSet / AXARIASetSize", "accesskit_macos does not read position_in_set / size_of_set")
    }
    let visible = rows ?? kids
    for (i, r) in visible.prefix(3).enumerated() {
        let place = placeOf(r, in: c)
        say(r, place: place ?? "(no place exposed; \(i + 1) of \(visible.count) built, by tree order)")
    }
}

// MARK: Focus by keyboard

/// Tab, posted to the app's process, as the keyboard sends it.
func tab(shift: Bool = false) {
    let src = CGEventSource(stateID: .hidSystemState)
    for down in [true, false] {
        let ev = CGEvent(keyboardEventSource: src, virtualKey: 0x30, keyDown: down)!
        if shift { ev.flags = .maskShift }
        ev.postToPid(pid)
    }
}

// MARK: Run

switch mode {
case "docs":
    print("== Input")
    if open("Input") {
        let fields = wait(5) { () -> [AXUIElement]? in let f = textFields(); return f.isEmpty ? nil : f }
        check(fields != nil, "the Input page has a text field", "\(fields?.count ?? 0)")
        if let f = fields?.first {
            checkField(f, sample: "hello brave world", multiline: false)
            // Focus moves on: to the next field by AXFocused, then by Tab.
            if let fields, fields.count > 1 {
                let g = fields[1]
                _ = set(g, "AXFocused", kCFBooleanTrue)
                let moved = wait(3) { same(attr(app, "AXFocusedUIElement"), g) ? true : nil } ?? false
                check(moved, "focus moves to the second field (AXFocused)")
                say(g)
                let was = attr(app, "AXFocusedUIElement")
                tab()
                let after = wait(3) { () -> AnyObject? in
                    let now = attr(app, "AXFocusedUIElement")
                    return now.flatMap { n in was.map { CFEqual(n, $0) } == true ? nil : n }
                }
                if let after {
                    check(true, "Tab posted to the process moves focus on")
                    say(after as! AXUIElement)
                } else {
                    info("Tab posted to the process did not move focus (a key event reaches an app only while it is active; not a failure of the tree)")
                }
            }
        }
    }
    print("== Textarea")
    if open("Textarea") {
        let area = wait(5) { textFields().first(where: { str($0, "AXRole") == "AXTextArea" }) }
        if let a = area {
            check(true, "the Textarea page has an AXTextArea")
            checkField(a, sample: "first line here\nsecond line there", multiline: true)
        } else {
            check(false, "the Textarea page has an AXTextArea", textFields().map { str($0, "AXRole") }.joined(separator: ", "))
        }
    }
    print("== Command (a list box)")
    if open("Command") {
        let list = wait(5) { find(window) { str($0, "AXRole") == "AXList" }.first }
        if let l = list {
            say(l)
            checkRows(container: l, rowRole: "AXStaticText", expectedCount: nil)
        } else {
            check(false, "the Command page has an AXList")
        }
    }
    print("== Table")
    if open("Table") {
        let table = wait(5) { find(window) { str($0, "AXRole") == "AXTable" }.first }
        if let t = table {
            say(t)
            checkRows(container: t, rowRole: "AXRow", expectedCount: nil)
        } else {
            check(false, "the Table page has an AXTable")
        }
    }
case "bigtable":
    print("== bigtable (1 000 000 rows, virtual)")
    let table = wait(10) { find(window) { str($0, "AXRole") == "AXTable" }.first }
    if let t = table {
        say(t)
        checkRows(container: t, rowRole: "AXRow", expectedCount: 1_000_000)
        let built = find(t, limit: 50000) { str($0, "AXRole") == "AXRow" }.count
        check(built > 0 && built < 1000, "only the rows on screen and pinned are in the tree", "\(built) AXRow nodes")
    } else {
        check(false, "the bigtable scene has an AXTable", find(window, limit: 200) { _ in true }.prefix(30).map { str($0, "AXRole") }.joined(separator: " "))
    }
default:
    print("unknown mode \(mode)")
    exit(2)
}

print("\(failed == 0 ? "ok" : "FAILED"): \(failed) failed, \(gaps) gaps (not mapped by accesskit_macos)")
exit(failed == 0 ? 0 : 1)
