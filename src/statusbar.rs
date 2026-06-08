#![allow(non_snake_case, unsafe_op_in_unsafe_fn)]

use std::time::Duration;

use muda::ContextMenu as _;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;

// ── Geometry (matches AppKit's NSPoint / NSSize / NSRect on 64-bit) ───────────

#[repr(C)] #[derive(Copy, Clone)] struct NSPoint { x: f64, y: f64 }
#[repr(C)] #[derive(Copy, Clone)] struct NSSize  { width: f64, height: f64 }
#[repr(C)] #[derive(Copy, Clone)] struct NSRect  { origin: NSPoint, size: NSSize }

unsafe impl Encode for NSPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl RefEncode for NSPoint {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

unsafe impl Encode for NSSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl RefEncode for NSSize {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

unsafe impl Encode for NSRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[NSPoint::ENCODING, NSSize::ENCODING]);
}
unsafe impl RefEncode for NSRect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

fn pt(x: f64, y: f64)              -> NSPoint { NSPoint { x, y } }
fn sz(w: f64, h: f64)              -> NSSize  { NSSize { width: w, height: h } }
fn rc(x: f64, y: f64, w: f64, h: f64) -> NSRect { NSRect { origin: pt(x, y), size: sz(w, h) } }

// ── Low-level ObjC helpers ────────────────────────────────────────────────────

fn cls(name: &str) -> &'static AnyClass {
    AnyClass::get(name).unwrap_or_else(|| panic!("ObjC class not found: {name}"))
}

unsafe fn nsstr(s: &str) -> *mut AnyObject {
    let alloc: *mut AnyObject = msg_send![cls("NSString"), alloc];
    msg_send![alloc, initWithBytes: s.as_ptr(), length: s.len(), encoding: 4usize]
}

unsafe fn sys_font(pt: f64) -> *mut AnyObject {
    msg_send![cls("NSFont"), systemFontOfSize: pt]
}

unsafe fn bold_font(pt: f64) -> *mut AnyObject {
    msg_send![cls("NSFont"), boldSystemFontOfSize: pt]
}

unsafe fn white_color() -> *mut AnyObject {
    msg_send![cls("NSColor"), whiteColor]
}

unsafe fn dark_color() -> *mut AnyObject {
    msg_send![cls("NSColor"), colorWithRed: 0.17f64, green: 0.17f64, blue: 0.17f64, alpha: 0.88f64]
}

unsafe fn clear_color() -> *mut AnyObject {
    msg_send![cls("NSColor"), clearColor]
}

// NSDictionary with NSFont + NSForegroundColor; caller owns the returned +1 ref.
unsafe fn text_attrs(font: *mut AnyObject, color: *mut AnyObject) -> *mut AnyObject {
    let d: *mut AnyObject = msg_send![cls("NSMutableDictionary"), new];
    let kf = unsafe { nsstr("NSFont") };
    let kc = unsafe { nsstr("NSColor") };
    let _: () = msg_send![d, setObject: font, forKey: kf];
    let _: () = msg_send![d, setObject: color, forKey: kc];
    let _: () = msg_send![kf, release];
    let _: () = msg_send![kc, release];
    d
}

// ── Box rendering ─────────────────────────────────────────────────────────────

const BOX_H:     f64 = 20.0;
const MIN_BOX_W: f64 = 44.0;
const CORNER:    f64 = 4.0;
const GAP:      f64 = 3.0;
const PAD_X:    f64 = 7.0;
const LABEL_PT: f64 = 7.5;
const VALUE_PT: f64 = 10.5;

pub struct BoxSpec<'a> {
    pub label: &'a str,
    pub value: &'a str,
}

// Renders all visible boxes side-by-side into a single NSImage.
// Returns a +1 retained NSImage — caller must release.
unsafe fn render_boxes(boxes: &[BoxSpec<'_>]) -> *mut AnyObject {
    let n = boxes.len();

    let lf = bold_font(LABEL_PT);
    let vf = sys_font(VALUE_PT);
    let wc = white_color();

    // Measure each value string before creating the image so we know its size.
    // sizeWithAttributes: is a pure text-metric call; no drawing context needed.
    let va_tmp = text_attrs(vf, wc);
    let widths: Vec<f64> = boxes
        .iter()
        .map(|b| {
            let vs = nsstr(b.value);
            let tsz: NSSize = msg_send![vs, sizeWithAttributes: va_tmp];
            let _: () = msg_send![vs, release];
            (PAD_X * 2.0 + tsz.width + 28.0).max(MIN_BOX_W)
        })
        .collect();
    let _: () = msg_send![va_tmp, release];

    let total_w = if n == 0 {
        1.0
    } else {
        widths.iter().sum::<f64>() + GAP * (n - 1) as f64
    };

    let alloc: *mut AnyObject = msg_send![cls("NSImage"), alloc];
    let image: *mut AnyObject = msg_send![alloc, initWithSize: sz(total_w, BOX_H)];

    let _: () = msg_send![image, lockFocus];

    let path_cls = cls("NSBezierPath");

    // Transparent background
    let _: () = msg_send![clear_color(), set];
    let _: () = msg_send![path_cls, fillRect: rc(0.0, 0.0, total_w, BOX_H)];

    let mut x = 0.0_f64;
    for (b, &bw) in boxes.iter().zip(widths.iter()) {
        // Rounded dark background
        let _: () = msg_send![dark_color(), setFill];
        let path: *mut AnyObject = msg_send![
            path_cls,
            bezierPathWithRoundedRect: rc(x, 0.0, bw, BOX_H),
            xRadius: CORNER,
            yRadius: CORNER
        ];
        let _: () = msg_send![path, fill];

        // Label — small, left-aligned
        let la = text_attrs(lf, wc);
        let ls = nsstr(b.label);
        let lsz: NSSize = msg_send![ls, sizeWithAttributes: la];
        let _: () = msg_send![ls, drawAtPoint: pt(x + PAD_X, BOX_H - lsz.height - 1.5), withAttributes: la];
        let _: () = msg_send![ls, release];
        let _: () = msg_send![la, release];

        // Value — larger, right-aligned, near bottom
        let va = text_attrs(vf, wc);
        let vs = nsstr(b.value);
        let vsz: NSSize = msg_send![vs, sizeWithAttributes: va];
        let vx = x + bw - PAD_X - vsz.width;
        let _: () = msg_send![vs, drawAtPoint: pt(vx, 2.5), withAttributes: va];
        let _: () = msg_send![vs, release];
        let _: () = msg_send![va, release];

        x += bw + GAP;
    }

    let _: () = msg_send![image, unlockFocus];
    image
}

// ── Poll-interval slider ──────────────────────────────────────────────────────

pub struct PollSlider {
    slider: *mut AnyObject, // non-owning; kept alive by the view hierarchy
    label:  *mut AnyObject,
}

unsafe impl Send for PollSlider {}

impl PollSlider {
    /// Builds a labelled slider NSMenuItem and appends it to the menu's NSMenu.
    pub unsafe fn append_to(menu: &muda::Menu) -> Self {
        const W:    f64 = 260.0;
        const H:    f64 = 46.0;
        const PAD:  f64 = 12.0;
        const INIT: f64 = 5.0;

        let ns_menu = menu.ns_menu() as *mut AnyObject;

        // ── "Refresh Rate" title ──────────────────────────────────────────────
        let a: *mut AnyObject = msg_send![cls("NSTextField"), alloc];
        let title_tf: *mut AnyObject = msg_send![a, initWithFrame: rc(PAD, 28.0, 100.0, 14.0)];
        let s = nsstr("Refresh Rate");
        let _: () = msg_send![title_tf, setStringValue: s];
        let _: () = msg_send![s, release];
        let _: () = msg_send![title_tf, setEditable: 0u8];
        let _: () = msg_send![title_tf, setBordered: 0u8];
        let _: () = msg_send![title_tf, setDrawsBackground: 0u8];
        let _: () = msg_send![title_tf, setFont: sys_font(11.0)];
        let c: *mut AnyObject = msg_send![cls("NSColor"), secondaryLabelColor];
        let _: () = msg_send![title_tf, setTextColor: c];

        // ── value label (updated each poll) ──────────────────────────────────
        let a: *mut AnyObject = msg_send![cls("NSTextField"), alloc];
        let val_tf: *mut AnyObject = msg_send![a, initWithFrame: rc(W - PAD - 42.0, 28.0, 42.0, 14.0)];
        let s = nsstr("5.0s");
        let _: () = msg_send![val_tf, setStringValue: s];
        let _: () = msg_send![s, release];
        let _: () = msg_send![val_tf, setEditable: 0u8];
        let _: () = msg_send![val_tf, setBordered: 0u8];
        let _: () = msg_send![val_tf, setDrawsBackground: 0u8];
        let _: () = msg_send![val_tf, setAlignment: 1usize]; // NSTextAlignmentRight
        let _: () = msg_send![val_tf, setFont: sys_font(11.0)];
        let c: *mut AnyObject = msg_send![cls("NSColor"), labelColor];
        let _: () = msg_send![val_tf, setTextColor: c];

        // ── slider ────────────────────────────────────────────────────────────
        let a: *mut AnyObject = msg_send![cls("NSSlider"), alloc];
        let slider: *mut AnyObject =
            msg_send![a, initWithFrame: rc(PAD, 6.0, W - PAD * 2.0, 20.0)];
        let _: () = msg_send![slider, setMinValue: 0.5f64];
        let _: () = msg_send![slider, setMaxValue: 10.0f64];
        let _: () = msg_send![slider, setDoubleValue: INIT];

        // ── container view ────────────────────────────────────────────────────
        let a: *mut AnyObject = msg_send![cls("NSView"), alloc];
        let view: *mut AnyObject = msg_send![a, initWithFrame: rc(0.0, 0.0, W, H)];
        let _: () = msg_send![view, addSubview: title_tf];
        let _: () = msg_send![title_tf, release];
        let _: () = msg_send![view, addSubview: val_tf];
        let _: () = msg_send![val_tf, release]; // view retains; we keep a non-owning ptr
        let _: () = msg_send![view, addSubview: slider];
        let _: () = msg_send![slider, release]; // same

        // ── NSMenuItem ────────────────────────────────────────────────────────
        let a: *mut AnyObject = msg_send![cls("NSMenuItem"), alloc];
        let item: *mut AnyObject = msg_send![a, init];
        let _: () = msg_send![item, setView: view];
        let _: () = msg_send![view, release];
        let _: () = msg_send![ns_menu, addItem: item];
        let _: () = msg_send![item, release];

        PollSlider { slider, label: val_tf }
    }

    /// Current slider position as a poll interval.
    pub fn interval(&self) -> Duration {
        let secs: f64 = unsafe { msg_send![self.slider, doubleValue] };
        Duration::from_secs_f64(secs.clamp(0.5, 10.0))
    }

    /// Refreshes the value label to match the slider's current position.
    /// Call once per poll so the label is correct next time the menu opens.
    pub fn sync_label(&self) {
        let secs: f64 = unsafe { msg_send![self.slider, doubleValue] };
        let text = if secs < 1.0 {
            format!("{}ms", (secs * 1000.0).round() as u32)
        } else {
            format!("{:.1}s", secs)
        };
        unsafe {
            let s = nsstr(&text);
            let _: () = msg_send![self.label, setStringValue: s];
            let _: () = msg_send![s, release];
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

pub struct StatusBar {
    item:   *mut AnyObject, // NSStatusItem — we hold +1 retain
    button: *mut AnyObject, // NSStatusBarButton — weak (owned by item)
}

// Safe: all AppKit calls happen on the main thread from about_to_wait.
unsafe impl Send for StatusBar {}

impl StatusBar {
    pub fn new(menu: &muda::Menu) -> Self {
        unsafe {
            let bar: *mut AnyObject = msg_send![cls("NSStatusBar"), systemStatusBar];

            // NSVariableStatusItemLength = -1
            let item: *mut AnyObject = msg_send![bar, statusItemWithLength: -1.0f64];
            let _: *mut AnyObject = msg_send![item, retain];

            let button: *mut AnyObject = msg_send![item, button];

            // NSImageOnly = 2  →  show only the image, no title text
            let _: () = msg_send![button, setImagePosition: 2usize];

            // Wire up the muda menu so clicking the item opens the dropdown
            let ns_menu = menu.ns_menu() as *mut AnyObject;
            let _: () = msg_send![item, setMenu: ns_menu];


            // Place an empty image so the item has non-zero width before the first poll
            let placeholder: *mut AnyObject = render_boxes(&[]);
            let _: () = msg_send![button, setImage: placeholder];
            let _: () = msg_send![placeholder, release];

            StatusBar { item, button }
        }
    }

    /// Call from the main thread (about_to_wait) after computing new stats.
    pub fn update(&self, boxes: &[BoxSpec<'_>]) {
        unsafe {
            // Hide the item entirely when nothing is enabled
            let visible: i8 = if boxes.is_empty() { 0 } else { 1 };
            let _: () = msg_send![self.item, setVisible: visible];

            if !boxes.is_empty() {
                let img = render_boxes(boxes);
                let _: () = msg_send![self.button, setImage: img];
                let _: () = msg_send![img, release];
            }
        }
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        unsafe {
            let bar: *mut AnyObject = msg_send![cls("NSStatusBar"), systemStatusBar];
            let _: () = msg_send![bar, removeStatusItem: self.item];
            let _: () = msg_send![self.item, release];
        }
    }
}
