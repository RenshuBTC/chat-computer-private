//! The document and its nodes as scripts use them (the DOM standard's Node, Element, Document, Text, Comment,
//! DocumentFragment, and HTML's element interfaces with their attributes reflected as properties): the tree walked
//! and changed, attributes, innerHTML and outerHTML (read with the page's own HTML parser, written as the standard
//! serializes), selectors (the page's own CSS selector matching), boxes' sizes from the last layout, and custom
//! elements (their classes' constructors run as elements of their names appear).

use super::events::{Change, ctor_of, handler_property, proto_from, record};
use super::*;
use crate::css;
use crate::dom::Element;
use crate::style;
use alloc::vec;
use js::object::ALL;

/// Each element name's interface (those not here are HTMLElement's, or HTMLUnknownElement's).
const ELEMENTS: &[(&str, &[&str])] = &[
    ("HTMLAnchorElement", &["a"]),
    ("HTMLAreaElement", &["area"]),
    ("HTMLAudioElement", &["audio"]),
    ("HTMLBaseElement", &["base"]),
    ("HTMLBodyElement", &["body"]),
    ("HTMLBRElement", &["br"]),
    ("HTMLButtonElement", &["button"]),
    ("HTMLCanvasElement", &["canvas"]),
    ("HTMLDataElement", &["data"]),
    ("HTMLDataListElement", &["datalist"]),
    ("HTMLDetailsElement", &["details"]),
    ("HTMLDialogElement", &["dialog"]),
    ("HTMLDivElement", &["div"]),
    ("HTMLDListElement", &["dl"]),
    ("HTMLEmbedElement", &["embed"]),
    ("HTMLFieldSetElement", &["fieldset"]),
    ("HTMLFormElement", &["form"]),
    ("HTMLHeadingElement", &["h1", "h2", "h3", "h4", "h5", "h6"]),
    ("HTMLHeadElement", &["head"]),
    ("HTMLHRElement", &["hr"]),
    ("HTMLHtmlElement", &["html"]),
    ("HTMLIFrameElement", &["iframe"]),
    ("HTMLImageElement", &["img"]),
    ("HTMLInputElement", &["input"]),
    ("HTMLLabelElement", &["label"]),
    ("HTMLLegendElement", &["legend"]),
    ("HTMLLIElement", &["li"]),
    ("HTMLLinkElement", &["link"]),
    ("HTMLMapElement", &["map"]),
    ("HTMLMetaElement", &["meta"]),
    ("HTMLMeterElement", &["meter"]),
    ("HTMLObjectElement", &["object"]),
    ("HTMLOListElement", &["ol"]),
    ("HTMLOptGroupElement", &["optgroup"]),
    ("HTMLOptionElement", &["option"]),
    ("HTMLOutputElement", &["output"]),
    ("HTMLParagraphElement", &["p"]),
    ("HTMLPictureElement", &["picture"]),
    ("HTMLPreElement", &["pre", "listing", "xmp"]),
    ("HTMLProgressElement", &["progress"]),
    ("HTMLQuoteElement", &["q", "blockquote"]),
    ("HTMLScriptElement", &["script"]),
    ("HTMLSelectElement", &["select"]),
    ("HTMLSlotElement", &["slot"]),
    ("HTMLSourceElement", &["source"]),
    ("HTMLSpanElement", &["span"]),
    ("HTMLStyleElement", &["style"]),
    ("HTMLTableCaptionElement", &["caption"]),
    ("HTMLTableCellElement", &["td", "th"]),
    ("HTMLTableColElement", &["col", "colgroup"]),
    ("HTMLTableElement", &["table"]),
    ("HTMLTableRowElement", &["tr"]),
    ("HTMLTableSectionElement", &["tbody", "thead", "tfoot"]),
    ("HTMLTemplateElement", &["template"]),
    ("HTMLTextAreaElement", &["textarea"]),
    ("HTMLTimeElement", &["time"]),
    ("HTMLTitleElement", &["title"]),
    ("HTMLTrackElement", &["track"]),
    ("HTMLUListElement", &["ul"]),
    ("HTMLVideoElement", &["video"]),
];

/// Elements that are HTMLElement itself (anything else unknown is HTMLUnknownElement).
const PLAIN: &[&str] = &[
    "abbr", "acronym", "address", "article", "aside", "b", "basefont", "bdi", "bdo", "big", "center", "cite", "code",
    "dd", "dfn", "dt", "em", "figcaption", "figure", "footer", "header", "hgroup", "i", "kbd", "main", "mark", "nav",
    "nobr", "noembed", "noframes", "noscript", "plaintext", "rb", "rp", "rt", "rtc", "ruby", "s", "samp", "search",
    "section", "small", "strike", "strong", "sub", "summary", "sup", "tt", "u", "var", "wbr", "menu",
];

/// SVG's elements (the ones that matter: anything inside <svg> is an SVGElement).
const SVG: &[(&str, &str)] = &[("svg", "SVGSVGElement"), ("path", "SVGPathElement"), ("g", "SVGGElement"), ("circle", "SVGCircleElement"), ("rect", "SVGRectElement"), ("use", "SVGUseElement")];

/// How a property reflects its attribute.
#[derive(Clone, Copy)]
enum Kind {
    /// The attribute's value ("" if it has none).
    Str,
    /// Whether it's there.
    Bool,
    /// Its value as an address, resolved against the page's.
    Url,
    /// A whole number (default if missing or not one).
    Long(i32),
    /// A whole number of at least 0.
    Unsigned(u32),
    /// Its value, or null if it has none.
    Nullable,
}

/// Properties reflecting attributes: (interface, property, attribute, kind).
const REFLECT: &[(&str, &str, &str, Kind)] = &[
    ("Element", "id", "id", Kind::Str),
    ("Element", "className", "class", Kind::Str),
    ("Element", "slot", "slot", Kind::Str),
    ("HTMLElement", "title", "title", Kind::Str),
    ("HTMLElement", "lang", "lang", Kind::Str),
    ("HTMLElement", "dir", "dir", Kind::Str),
    ("HTMLElement", "hidden", "hidden", Kind::Bool),
    ("HTMLElement", "accessKey", "accesskey", Kind::Str),
    ("HTMLElement", "inputMode", "inputmode", Kind::Str),
    ("HTMLElement", "enterKeyHint", "enterkeyhint", Kind::Str),
    ("HTMLElement", "nonce", "nonce", Kind::Str),
    ("HTMLElement", "autofocus", "autofocus", Kind::Bool),
    ("HTMLElement", "inert", "inert", Kind::Bool),
    ("HTMLElement", "popover", "popover", Kind::Nullable),
    ("HTMLAnchorElement", "target", "target", Kind::Str),
    ("HTMLAnchorElement", "rel", "rel", Kind::Str),
    ("HTMLAnchorElement", "download", "download", Kind::Str),
    ("HTMLAnchorElement", "hreflang", "hreflang", Kind::Str),
    ("HTMLAnchorElement", "type", "type", Kind::Str),
    ("HTMLAnchorElement", "ping", "ping", Kind::Str),
    ("HTMLAnchorElement", "referrerPolicy", "referrerpolicy", Kind::Str),
    ("HTMLAreaElement", "alt", "alt", Kind::Str),
    ("HTMLAreaElement", "coords", "coords", Kind::Str),
    ("HTMLAreaElement", "shape", "shape", Kind::Str),
    ("HTMLAreaElement", "target", "target", Kind::Str),
    ("HTMLAreaElement", "rel", "rel", Kind::Str),
    ("HTMLBaseElement", "target", "target", Kind::Str),
    ("HTMLButtonElement", "disabled", "disabled", Kind::Bool),
    ("HTMLButtonElement", "name", "name", Kind::Str),
    ("HTMLButtonElement", "value", "value", Kind::Str),
    ("HTMLButtonElement", "formAction", "formaction", Kind::Url),
    ("HTMLButtonElement", "formMethod", "formmethod", Kind::Str),
    ("HTMLButtonElement", "formNoValidate", "formnovalidate", Kind::Bool),
    ("HTMLButtonElement", "formTarget", "formtarget", Kind::Str),
    ("HTMLCanvasElement", "width", "width", Kind::Unsigned(300)),
    ("HTMLCanvasElement", "height", "height", Kind::Unsigned(150)),
    ("HTMLDataElement", "value", "value", Kind::Str),
    ("HTMLDetailsElement", "open", "open", Kind::Bool),
    ("HTMLDetailsElement", "name", "name", Kind::Str),
    ("HTMLDialogElement", "open", "open", Kind::Bool),
    ("HTMLEmbedElement", "src", "src", Kind::Url),
    ("HTMLEmbedElement", "type", "type", Kind::Str),
    ("HTMLEmbedElement", "width", "width", Kind::Str),
    ("HTMLEmbedElement", "height", "height", Kind::Str),
    ("HTMLFieldSetElement", "disabled", "disabled", Kind::Bool),
    ("HTMLFieldSetElement", "name", "name", Kind::Str),
    ("HTMLFormElement", "acceptCharset", "accept-charset", Kind::Str),
    ("HTMLFormElement", "autocomplete", "autocomplete", Kind::Str),
    ("HTMLFormElement", "enctype", "enctype", Kind::Str),
    ("HTMLFormElement", "name", "name", Kind::Str),
    ("HTMLFormElement", "noValidate", "novalidate", Kind::Bool),
    ("HTMLFormElement", "target", "target", Kind::Str),
    ("HTMLFormElement", "rel", "rel", Kind::Str),
    ("HTMLIFrameElement", "src", "src", Kind::Url),
    ("HTMLIFrameElement", "srcdoc", "srcdoc", Kind::Str),
    ("HTMLIFrameElement", "name", "name", Kind::Str),
    ("HTMLIFrameElement", "allow", "allow", Kind::Str),
    ("HTMLIFrameElement", "allowFullscreen", "allowfullscreen", Kind::Bool),
    ("HTMLIFrameElement", "width", "width", Kind::Str),
    ("HTMLIFrameElement", "height", "height", Kind::Str),
    ("HTMLIFrameElement", "loading", "loading", Kind::Str),
    ("HTMLIFrameElement", "referrerPolicy", "referrerpolicy", Kind::Str),
    ("HTMLImageElement", "alt", "alt", Kind::Str),
    ("HTMLImageElement", "src", "src", Kind::Url),
    ("HTMLImageElement", "srcset", "srcset", Kind::Str),
    ("HTMLImageElement", "sizes", "sizes", Kind::Str),
    ("HTMLImageElement", "crossOrigin", "crossorigin", Kind::Nullable),
    ("HTMLImageElement", "useMap", "usemap", Kind::Str),
    ("HTMLImageElement", "isMap", "ismap", Kind::Bool),
    ("HTMLImageElement", "loading", "loading", Kind::Str),
    ("HTMLImageElement", "decoding", "decoding", Kind::Str),
    ("HTMLImageElement", "referrerPolicy", "referrerpolicy", Kind::Str),
    ("HTMLImageElement", "fetchPriority", "fetchpriority", Kind::Str),
    ("HTMLInputElement", "accept", "accept", Kind::Str),
    ("HTMLInputElement", "alt", "alt", Kind::Str),
    ("HTMLInputElement", "autocomplete", "autocomplete", Kind::Str),
    ("HTMLInputElement", "defaultChecked", "checked", Kind::Bool),
    ("HTMLInputElement", "defaultValue", "value", Kind::Str),
    ("HTMLInputElement", "dirName", "dirname", Kind::Str),
    ("HTMLInputElement", "disabled", "disabled", Kind::Bool),
    ("HTMLInputElement", "formAction", "formaction", Kind::Url),
    ("HTMLInputElement", "formMethod", "formmethod", Kind::Str),
    ("HTMLInputElement", "formNoValidate", "formnovalidate", Kind::Bool),
    ("HTMLInputElement", "formTarget", "formtarget", Kind::Str),
    ("HTMLInputElement", "max", "max", Kind::Str),
    ("HTMLInputElement", "maxLength", "maxlength", Kind::Long(-1)),
    ("HTMLInputElement", "min", "min", Kind::Str),
    ("HTMLInputElement", "minLength", "minlength", Kind::Long(-1)),
    ("HTMLInputElement", "multiple", "multiple", Kind::Bool),
    ("HTMLInputElement", "name", "name", Kind::Str),
    ("HTMLInputElement", "pattern", "pattern", Kind::Str),
    ("HTMLInputElement", "placeholder", "placeholder", Kind::Str),
    ("HTMLInputElement", "readOnly", "readonly", Kind::Bool),
    ("HTMLInputElement", "required", "required", Kind::Bool),
    ("HTMLInputElement", "size", "size", Kind::Unsigned(20)),
    ("HTMLInputElement", "src", "src", Kind::Url),
    ("HTMLInputElement", "step", "step", Kind::Str),
    ("HTMLLabelElement", "htmlFor", "for", Kind::Str),
    ("HTMLLIElement", "value", "value", Kind::Long(0)),
    ("HTMLLinkElement", "href", "href", Kind::Url),
    ("HTMLLinkElement", "rel", "rel", Kind::Str),
    ("HTMLLinkElement", "media", "media", Kind::Str),
    ("HTMLLinkElement", "hreflang", "hreflang", Kind::Str),
    ("HTMLLinkElement", "type", "type", Kind::Str),
    ("HTMLLinkElement", "as", "as", Kind::Str),
    ("HTMLLinkElement", "crossOrigin", "crossorigin", Kind::Nullable),
    ("HTMLLinkElement", "integrity", "integrity", Kind::Str),
    ("HTMLLinkElement", "disabled", "disabled", Kind::Bool),
    ("HTMLLinkElement", "sizes", "sizes", Kind::Str),
    ("HTMLLinkElement", "fetchPriority", "fetchpriority", Kind::Str),
    ("HTMLLinkElement", "referrerPolicy", "referrerpolicy", Kind::Str),
    ("HTMLMapElement", "name", "name", Kind::Str),
    ("HTMLMetaElement", "name", "name", Kind::Str),
    ("HTMLMetaElement", "content", "content", Kind::Str),
    ("HTMLMetaElement", "httpEquiv", "http-equiv", Kind::Str),
    ("HTMLMetaElement", "media", "media", Kind::Str),
    ("HTMLMeterElement", "value", "value", Kind::Str),
    ("HTMLMeterElement", "min", "min", Kind::Str),
    ("HTMLMeterElement", "max", "max", Kind::Str),
    ("HTMLObjectElement", "data", "data", Kind::Url),
    ("HTMLObjectElement", "type", "type", Kind::Str),
    ("HTMLObjectElement", "name", "name", Kind::Str),
    ("HTMLObjectElement", "width", "width", Kind::Str),
    ("HTMLObjectElement", "height", "height", Kind::Str),
    ("HTMLOListElement", "reversed", "reversed", Kind::Bool),
    ("HTMLOListElement", "start", "start", Kind::Long(1)),
    ("HTMLOListElement", "type", "type", Kind::Str),
    ("HTMLOptGroupElement", "disabled", "disabled", Kind::Bool),
    ("HTMLOptGroupElement", "label", "label", Kind::Str),
    ("HTMLOptionElement", "disabled", "disabled", Kind::Bool),
    ("HTMLOptionElement", "defaultSelected", "selected", Kind::Bool),
    ("HTMLOutputElement", "name", "name", Kind::Str),
    ("HTMLQuoteElement", "cite", "cite", Kind::Url),
    ("HTMLScriptElement", "src", "src", Kind::Url),
    ("HTMLScriptElement", "type", "type", Kind::Str),
    ("HTMLScriptElement", "charset", "charset", Kind::Str),
    ("HTMLScriptElement", "defer", "defer", Kind::Bool),
    ("HTMLScriptElement", "noModule", "nomodule", Kind::Bool),
    ("HTMLScriptElement", "crossOrigin", "crossorigin", Kind::Nullable),
    ("HTMLScriptElement", "integrity", "integrity", Kind::Str),
    ("HTMLScriptElement", "referrerPolicy", "referrerpolicy", Kind::Str),
    ("HTMLScriptElement", "event", "event", Kind::Str),
    ("HTMLScriptElement", "htmlFor", "for", Kind::Str),
    ("HTMLSelectElement", "disabled", "disabled", Kind::Bool),
    ("HTMLSelectElement", "multiple", "multiple", Kind::Bool),
    ("HTMLSelectElement", "name", "name", Kind::Str),
    ("HTMLSelectElement", "required", "required", Kind::Bool),
    ("HTMLSelectElement", "size", "size", Kind::Unsigned(0)),
    ("HTMLSelectElement", "autocomplete", "autocomplete", Kind::Str),
    ("HTMLSlotElement", "name", "name", Kind::Str),
    ("HTMLSourceElement", "src", "src", Kind::Url),
    ("HTMLSourceElement", "srcset", "srcset", Kind::Str),
    ("HTMLSourceElement", "sizes", "sizes", Kind::Str),
    ("HTMLSourceElement", "media", "media", Kind::Str),
    ("HTMLSourceElement", "type", "type", Kind::Str),
    ("HTMLStyleElement", "media", "media", Kind::Str),
    ("HTMLStyleElement", "type", "type", Kind::Str),
    ("HTMLTableCellElement", "colSpan", "colspan", Kind::Unsigned(1)),
    ("HTMLTableCellElement", "rowSpan", "rowspan", Kind::Unsigned(1)),
    ("HTMLTableCellElement", "headers", "headers", Kind::Str),
    ("HTMLTableCellElement", "scope", "scope", Kind::Str),
    ("HTMLTableCellElement", "abbr", "abbr", Kind::Str),
    ("HTMLTableColElement", "span", "span", Kind::Unsigned(1)),
    ("HTMLTextAreaElement", "autocomplete", "autocomplete", Kind::Str),
    ("HTMLTextAreaElement", "cols", "cols", Kind::Unsigned(20)),
    ("HTMLTextAreaElement", "dirName", "dirname", Kind::Str),
    ("HTMLTextAreaElement", "disabled", "disabled", Kind::Bool),
    ("HTMLTextAreaElement", "maxLength", "maxlength", Kind::Long(-1)),
    ("HTMLTextAreaElement", "minLength", "minlength", Kind::Long(-1)),
    ("HTMLTextAreaElement", "name", "name", Kind::Str),
    ("HTMLTextAreaElement", "placeholder", "placeholder", Kind::Str),
    ("HTMLTextAreaElement", "readOnly", "readonly", Kind::Bool),
    ("HTMLTextAreaElement", "required", "required", Kind::Bool),
    ("HTMLTextAreaElement", "rows", "rows", Kind::Unsigned(2)),
    ("HTMLTextAreaElement", "wrap", "wrap", Kind::Str),
    ("HTMLTimeElement", "dateTime", "datetime", Kind::Str),
    ("HTMLTrackElement", "kind", "kind", Kind::Str),
    ("HTMLTrackElement", "src", "src", Kind::Url),
    ("HTMLTrackElement", "srclang", "srclang", Kind::Str),
    ("HTMLTrackElement", "label", "label", Kind::Str),
    ("HTMLTrackElement", "default", "default", Kind::Bool),
    ("HTMLMediaElement", "src", "src", Kind::Url),
    ("HTMLMediaElement", "crossOrigin", "crossorigin", Kind::Nullable),
    ("HTMLMediaElement", "preload", "preload", Kind::Str),
    ("HTMLMediaElement", "autoplay", "autoplay", Kind::Bool),
    ("HTMLMediaElement", "loop", "loop", Kind::Bool),
    ("HTMLMediaElement", "controls", "controls", Kind::Bool),
    ("HTMLMediaElement", "defaultMuted", "muted", Kind::Bool),
    ("HTMLVideoElement", "width", "width", Kind::Unsigned(0)),
    ("HTMLVideoElement", "height", "height", Kind::Unsigned(0)),
    ("HTMLVideoElement", "poster", "poster", Kind::Url),
    ("HTMLVideoElement", "playsInline", "playsinline", Kind::Bool),
    ("HTMLProgressElement", "max", "max", Kind::Str),
];

pub fn init(vm: &mut Vm) {
    // Node.
    let np = interface(vm, "Node", Some("EventTarget"), 0, None);
    for (i, name) in ["ELEMENT_NODE", "ATTRIBUTE_NODE", "TEXT_NODE", "CDATA_SECTION_NODE", "ENTITY_REFERENCE_NODE", "ENTITY_NODE", "PROCESSING_INSTRUCTION_NODE", "COMMENT_NODE", "DOCUMENT_NODE", "DOCUMENT_TYPE_NODE", "DOCUMENT_FRAGMENT_NODE", "NOTATION_NODE"].iter().enumerate() {
        constant(vm, np, name, (i + 1) as f64);
    }
    for (name, v) in [("DOCUMENT_POSITION_DISCONNECTED", 1.0), ("DOCUMENT_POSITION_PRECEDING", 2.0), ("DOCUMENT_POSITION_FOLLOWING", 4.0), ("DOCUMENT_POSITION_CONTAINS", 8.0), ("DOCUMENT_POSITION_CONTAINED_BY", 16.0), ("DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC", 32.0)] {
        constant(vm, np, name, v);
    }
    attribute(vm, np, "nodeType", node_type, None);
    attribute(vm, np, "nodeName", node_name, None);
    attribute(vm, np, "nodeValue", node_value, Some(set_node_value));
    attribute(vm, np, "textContent", text_content, Some(set_text_content));
    attribute(vm, np, "parentNode", parent_node, None);
    attribute(vm, np, "parentElement", parent_element, None);
    attribute(vm, np, "firstChild", first_child, None);
    attribute(vm, np, "lastChild", last_child, None);
    attribute(vm, np, "previousSibling", previous_sibling, None);
    attribute(vm, np, "nextSibling", next_sibling, None);
    attribute(vm, np, "ownerDocument", owner_document, None);
    attribute(vm, np, "isConnected", is_connected, None);
    attribute(vm, np, "baseURI", base_uri, None);
    attribute(vm, np, "childNodes", child_nodes, None);
    method(vm, np, "appendChild", 1, append_child);
    method(vm, np, "insertBefore", 2, insert_before);
    method(vm, np, "removeChild", 1, remove_child);
    method(vm, np, "replaceChild", 2, replace_child);
    method(vm, np, "cloneNode", 0, clone_node);
    method(vm, np, "contains", 1, contains);
    method(vm, np, "hasChildNodes", 0, has_child_nodes);
    method(vm, np, "getRootNode", 0, get_root_node);
    method(vm, np, "compareDocumentPosition", 1, compare_position);
    method(vm, np, "isSameNode", 1, is_same_node);
    method(vm, np, "isEqualNode", 1, is_equal_node);
    method(vm, np, "normalize", 0, normalize);
    method(vm, np, "lookupNamespaceURI", 1, null_fn);
    method(vm, np, "lookupPrefix", 1, null_fn);
    method(vm, np, "isDefaultNamespace", 1, false_fn);

    // CharacterData, Text, Comment.
    let cd = interface(vm, "CharacterData", Some("Node"), 0, None);
    attribute(vm, cd, "data", node_value, Some(set_node_value));
    attribute(vm, cd, "length", data_length, None);
    method(vm, cd, "appendData", 1, append_data);
    method(vm, cd, "insertData", 2, insert_data);
    method(vm, cd, "deleteData", 2, delete_data);
    method(vm, cd, "replaceData", 3, replace_data);
    method(vm, cd, "substringData", 2, substring_data);
    child_node_methods(vm, cd);
    let tp = interface(vm, "Text", Some("CharacterData"), 0, Some(text_new));
    attribute(vm, tp, "wholeText", whole_text, None);
    method(vm, tp, "splitText", 1, split_text);
    interface(vm, "CDATASection", Some("Text"), 0, None);
    interface(vm, "Comment", Some("CharacterData"), 0, Some(comment_new));
    interface(vm, "ProcessingInstruction", Some("CharacterData"), 0, None);
    let dt = interface(vm, "DocumentType", Some("Node"), 0, None);
    for name in ["name", "publicId", "systemId"] {
        attribute_with(vm, dt, name, doctype_part, None, js(name));
    }

    // Element.
    let ep = interface(vm, "Element", Some("Node"), 0, None);
    attribute(vm, ep, "tagName", tag_name, None);
    attribute(vm, ep, "localName", local_name, None);
    attribute(vm, ep, "namespaceURI", namespace_uri, None);
    attribute(vm, ep, "prefix", null_fn, None);
    attribute(vm, ep, "attributes", attributes, None);
    attribute(vm, ep, "innerHTML", inner_html, Some(set_inner_html));
    attribute(vm, ep, "outerHTML", outer_html, Some(set_outer_html));
    attribute(vm, ep, "shadowRoot", shadow_root, None);
    attribute(vm, ep, "scrollTop", scroll_top, Some(set_scroll_top));
    attribute(vm, ep, "scrollLeft", zero_fn, Some(ignore_fn));
    attribute(vm, ep, "scrollWidth", scroll_width, None);
    attribute(vm, ep, "scrollHeight", scroll_height, None);
    attribute(vm, ep, "clientTop", zero_fn, None);
    attribute(vm, ep, "clientLeft", zero_fn, None);
    attribute(vm, ep, "clientWidth", client_width, None);
    attribute(vm, ep, "clientHeight", client_height, None);
    method(vm, ep, "getAttribute", 1, get_attribute);
    method(vm, ep, "getAttributeNS", 2, get_attribute_ns);
    method(vm, ep, "setAttribute", 2, set_attribute);
    method(vm, ep, "setAttributeNS", 3, set_attribute_ns);
    method(vm, ep, "removeAttribute", 1, remove_attribute);
    method(vm, ep, "removeAttributeNS", 2, remove_attribute_ns);
    method(vm, ep, "hasAttribute", 1, has_attribute);
    method(vm, ep, "hasAttributeNS", 2, has_attribute_ns);
    method(vm, ep, "hasAttributes", 0, has_attributes);
    method(vm, ep, "toggleAttribute", 1, toggle_attribute);
    method(vm, ep, "getAttributeNames", 0, get_attribute_names);
    method(vm, ep, "getAttributeNode", 1, get_attribute_node);
    method(vm, ep, "insertAdjacentHTML", 2, insert_adjacent_html);
    method(vm, ep, "insertAdjacentElement", 2, insert_adjacent_element);
    method(vm, ep, "insertAdjacentText", 2, insert_adjacent_text);
    method(vm, ep, "matches", 1, matches);
    method(vm, ep, "webkitMatchesSelector", 1, matches);
    method(vm, ep, "msMatchesSelector", 1, matches);
    method(vm, ep, "closest", 1, closest);
    method(vm, ep, "getBoundingClientRect", 0, bounding_rect);
    method(vm, ep, "getClientRects", 0, client_rects);
    method(vm, ep, "scrollIntoView", 0, scroll_into_view);
    method(vm, ep, "scrollTo", 0, ignore_fn);
    method(vm, ep, "scrollBy", 0, ignore_fn);
    method(vm, ep, "scroll", 0, ignore_fn);
    method(vm, ep, "attachShadow", 1, attach_shadow);
    method(vm, ep, "setPointerCapture", 1, ignore_fn);
    method(vm, ep, "releasePointerCapture", 1, ignore_fn);
    method(vm, ep, "hasPointerCapture", 1, false_fn);
    method(vm, ep, "requestFullscreen", 0, rejected_fn);
    method(vm, ep, "requestPointerLock", 0, ignore_fn);
    method(vm, ep, "animate", 2, animate);
    method(vm, ep, "getAnimations", 0, empty_array);
    method(vm, ep, "checkVisibility", 0, true_fn);
    parent_node_methods(vm, ep);
    child_node_methods(vm, ep);
    sibling_element_getters(vm, ep);
    method(vm, ep, "getElementsByTagName", 1, get_elements_by_tag_name);
    method(vm, ep, "getElementsByTagNameNS", 2, get_elements_by_tag_name_ns);
    method(vm, ep, "getElementsByClassName", 1, get_elements_by_class_name);

    // HTMLElement and its kinds.
    let hp = interface(vm, "HTMLElement", Some("Element"), 0, Some(html_element_new));
    attribute(vm, hp, "innerText", text_content, Some(set_text_content));
    attribute(vm, hp, "outerText", text_content, Some(set_outer_html_text));
    attribute(vm, hp, "tabIndex", tab_index, Some(set_tab_index));
    attribute(vm, hp, "draggable", draggable, Some(set_draggable));
    attribute(vm, hp, "spellcheck", true_fn, Some(ignore_fn));
    attribute(vm, hp, "translate", true_fn, Some(ignore_fn));
    attribute(vm, hp, "contentEditable", content_editable, Some(set_content_editable));
    attribute(vm, hp, "isContentEditable", is_content_editable, None);
    attribute(vm, hp, "offsetParent", offset_parent, None);
    attribute(vm, hp, "offsetTop", offset_top, None);
    attribute(vm, hp, "offsetLeft", offset_left, None);
    attribute(vm, hp, "offsetWidth", offset_width, None);
    attribute(vm, hp, "offsetHeight", offset_height, None);
    method(vm, hp, "click", 0, click);
    method(vm, hp, "focus", 0, focus);
    method(vm, hp, "blur", 0, blur);
    method(vm, hp, "showPopover", 0, ignore_fn);
    method(vm, hp, "hidePopover", 0, ignore_fn);
    method(vm, hp, "togglePopover", 0, false_fn);
    method(vm, hp, "attachInternals", 0, attach_internals);
    for name in super::events::HANDLERS {
        handler_property(vm, hp, name);
    }
    interface(vm, "HTMLUnknownElement", Some("HTMLElement"), 0, None);
    interface(vm, "HTMLMediaElement", Some("HTMLElement"), 0, None);
    for (name, _) in ELEMENTS {
        let parent = if matches!(*name, "HTMLAudioElement" | "HTMLVideoElement") { "HTMLMediaElement" } else { "HTMLElement" };
        interface(vm, name, Some(parent), 0, None);
    }
    interface(vm, "SVGElement", Some("Element"), 0, None);
    interface(vm, "SVGGraphicsElement", Some("SVGElement"), 0, None);
    for (_, iface) in SVG {
        interface(vm, iface, Some("SVGGraphicsElement"), 0, None);
    }
    let sp = host_ref(vm).proto("SVGElement").expect("made");
    attribute(vm, sp, "style", super::lists::style, Some(super::lists::set_style));
    attribute(vm, sp, "dataset", super::lists::dataset, None);
    for name in super::events::HANDLERS {
        handler_property(vm, sp, name);
    }
    for (iface, prop, attr, kind) in REFLECT {
        let Some(p) = host_ref(vm).proto(iface) else { continue };
        let data = js(attr);
        let (get, set): (NativeFn, NativeFn) = match kind {
            Kind::Str => (reflect_str, reflect_set_str),
            Kind::Bool => (reflect_bool, reflect_set_bool),
            Kind::Url => (reflect_url, reflect_set_str),
            Kind::Nullable => (reflect_nullable, reflect_set_nullable),
            Kind::Long(_) | Kind::Unsigned(_) => (reflect_num, reflect_set_num),
        };
        attribute_with(vm, p, prop, get, Some(set), data.clone());
        if let Kind::Long(d) = kind {
            set_default(vm, p, prop, *d as f64);
        }
        if let Kind::Unsigned(d) = kind {
            set_default(vm, p, prop, *d as f64);
        }
    }
    element_specifics(vm);

    // Document and DocumentFragment.
    let dp = interface(vm, "Document", Some("Node"), 0, Some(document_new));
    interface(vm, "HTMLDocument", Some("Document"), 0, None);
    interface(vm, "XMLDocument", Some("Document"), 0, None);
    document_members(vm, dp);
    let fp = interface(vm, "DocumentFragment", Some("Node"), 0, Some(fragment_new));
    parent_node_methods(vm, fp);
    method(vm, fp, "getElementById", 1, get_element_by_id);
    let sr = interface(vm, "ShadowRoot", Some("DocumentFragment"), 0, None);
    attribute(vm, sr, "host", shadow_host, None);
    attribute(vm, sr, "mode", shadow_mode, None);
    attribute(vm, sr, "innerHTML", inner_html, Some(set_inner_html));
    attribute(vm, sr, "activeElement", null_fn, None);
    let ap = interface(vm, "Attr", Some("Node"), 0, None);
    attribute(vm, ap, "name", attr_name, None);
    attribute(vm, ap, "localName", attr_name, None);
    attribute(vm, ap, "value", attr_value, Some(set_attr_value));
    attribute(vm, ap, "ownerElement", attr_owner, None);
    attribute(vm, ap, "specified", true_fn, None);
    attribute(vm, ap, "namespaceURI", null_fn, None);
    attribute(vm, ap, "prefix", null_fn, None);
    let rp = interface(vm, "DOMRect", Some("DOMRectReadOnly"), 0, Some(dom_rect_new));
    let _ = rp;
    interface(vm, "DOMRectReadOnly", None, 0, None);
    // (DOMRect's prototype made before its parent's: put the chain right.)
    let rro = host_ref(vm).proto("DOMRectReadOnly").expect("made");
    vm.heap.get_mut(rp).proto = Some(rro);
    method(vm, rro, "toJSON", 0, rect_to_json);
    interface(vm, "MutationRecord", None, 0, None);
    // Custom elements.
    let cr = interface(vm, "CustomElementRegistry", None, 0, None);
    method(vm, cr, "define", 2, ce_define);
    method(vm, cr, "get", 1, ce_get);
    method(vm, cr, "getName", 1, ce_get_name);
    method(vm, cr, "whenDefined", 1, ce_when_defined);
    method(vm, cr, "upgrade", 1, ce_upgrade);
    let reg = vm.alloc(Some(cr), Class::Ordinary);
    let w = host_ref(vm).window;
    vm.put(w, "customElements", Value::Obj(reg), HIDDEN);
    let ei = interface(vm, "ElementInternals", None, 0, None);
    method(vm, ei, "setFormValue", 1, ignore_fn);
    method(vm, ei, "setValidity", 1, ignore_fn);
    method(vm, ei, "checkValidity", 0, true_fn);
    method(vm, ei, "reportValidity", 0, true_fn);
    attribute(vm, ei, "states", element_states, None);
    // Ranges and tree walkers, as far as pages lean on them.
    let rg = interface(vm, "Range", None, 0, Some(range_new));
    for name in ["setStart", "setEnd", "setStartBefore", "setStartAfter", "setEndBefore", "setEndAfter", "selectNode", "selectNodeContents", "collapse", "detach", "insertNode", "surroundContents"] {
        method(vm, rg, name, 2, ignore_fn);
    }
    method(vm, rg, "getBoundingClientRect", 0, empty_rect);
    method(vm, rg, "getClientRects", 0, empty_array);
    method(vm, rg, "cloneRange", 0, range_new_fn);
    method(vm, rg, "createContextualFragment", 1, create_contextual_fragment);
    method(vm, rg, "toString", 0, empty_string);
    attribute(vm, rg, "collapsed", true_fn, None);
    let tw = interface(vm, "TreeWalker", None, 0, None);
    attribute(vm, tw, "currentNode", walker_current, Some(walker_set_current));
    attribute(vm, tw, "root", walker_root, None);
    method(vm, tw, "nextNode", 0, walker_next);
    method(vm, tw, "previousNode", 0, walker_previous);
    method(vm, tw, "parentNode", 0, walker_parent);
    method(vm, tw, "firstChild", 0, walker_first_child);
    method(vm, tw, "nextSibling", 0, walker_next_sibling);
    let ni = interface(vm, "NodeIterator", None, 0, None);
    method(vm, ni, "nextNode", 0, walker_next);
    method(vm, ni, "previousNode", 0, walker_previous);
    method(vm, ni, "detach", 0, ignore_fn);
    let nf = interface(vm, "NodeFilter", None, 0, None);
    for (name, v) in [("FILTER_ACCEPT", 1.0), ("FILTER_REJECT", 2.0), ("FILTER_SKIP", 3.0), ("SHOW_ALL", 4294967295.0), ("SHOW_ELEMENT", 1.0), ("SHOW_ATTRIBUTE", 2.0), ("SHOW_TEXT", 4.0), ("SHOW_CDATA_SECTION", 8.0), ("SHOW_PROCESSING_INSTRUCTION", 64.0), ("SHOW_COMMENT", 128.0), ("SHOW_DOCUMENT", 256.0), ("SHOW_DOCUMENT_TYPE", 512.0), ("SHOW_DOCUMENT_FRAGMENT", 1024.0)] {
        constant(vm, nf, name, v);
    }
    let dom_parser = interface(vm, "DOMParser", None, 0, Some(dom_parser_new));
    method(vm, dom_parser, "parseFromString", 2, parse_from_string);
    let xs = interface(vm, "XMLSerializer", None, 0, Some(xml_serializer_new));
    method(vm, xs, "serializeToString", 1, serialize_to_string);
    let w = host_ref(vm).window;
    let img = vm.native_with("Image", 0, illegal_call, Some(image_new), Value::Undefined);
    if let Some(p) = host_ref(vm).proto("HTMLImageElement") {
        vm.put_key(img, Key::Str(js::names::PROTOTYPE), Value::Obj(p), 0);
    }
    vm.put(w, "Image", Value::Obj(img), HIDDEN);
    let opt = vm.native_with("Option", 0, illegal_call, Some(option_new), Value::Undefined);
    if let Some(p) = host_ref(vm).proto("HTMLOptionElement") {
        vm.put_key(opt, Key::Str(js::names::PROTOTYPE), Value::Obj(p), 0);
    }
    vm.put(w, "Option", Value::Obj(opt), HIDDEN);
}

/// A reflected number's default, kept on its getter.
fn set_default(vm: &mut Vm, proto: Gc, prop: &str, d: f64) {
    let a = vm.atom(prop);
    if let Some(Prop { slot: Slot::Accessor(Value::Obj(g), _), .. }) = vm.heap.get(proto).props.get(&Key::Str(a)).cloned() {
        vm.set_data2(g, Value::Num(d));
    }
}

fn illegal_call(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Err(vm.type_error("Please use the 'new' operator, this DOM object constructor cannot be called as a function."))
}

/// The prototype for a node's object (by what it is; an element by its name).
pub fn proto_for(vm: &mut Vm, id: NodeId) -> Gc {
    let h = host_ref(vm);
    let name = match &h.doc.nodes.get(id).map(|n| &n.data) {
        Some(Data::Document) => "HTMLDocument",
        Some(Data::Text(_)) => "Text",
        Some(Data::Comment(_)) => "Comment",
        Some(Data::Fragment) => {
            if h.doc.shadows.iter().any(|(_, f)| *f == id) { "ShadowRoot" } else { "DocumentFragment" }
        }
        Some(Data::Element(e)) => {
            if let Some((_, c)) = h.custom.iter().find(|(n, _)| *n == e.name) {
                let c = c.clone();
                if let Value::Obj(cg) = c
                    && let Some(Prop { slot: Slot::Data(Value::Obj(p)), .. }) = vm.heap.get(cg).props.get(&Key::Str(js::names::PROTOTYPE)).cloned()
                {
                    return p;
                }
            }
            interface_name(&h.doc, id, &e.name)
        }
        None => "Node",
    };
    host_ref(vm).proto(name).unwrap_or_else(|| vm.proto(I::ObjectProto))
}

fn interface_name(doc: &Document, id: NodeId, name: &str) -> &'static str {
    if let Some((iface, _)) = ELEMENTS.iter().find(|(_, names)| names.contains(&name)) {
        return iface;
    }
    // Inside an <svg>: SVG's.
    let mut at = doc.parent(id);
    let in_svg = name == "svg" || core::iter::from_fn(|| {
        let p = at?;
        at = doc.parent(p);
        Some(p)
    })
    .any(|p| doc.name(p) == "svg");
    if in_svg {
        return SVG.iter().find(|(n, _)| *n == name).map_or("SVGElement", |(_, i)| i);
    }
    if PLAIN.contains(&name) || name.contains('-') {
        return "HTMLElement";
    }
    "HTMLUnknownElement"
}

// ------------------------------------------------------------------------------------------------- helpers

fn null_fn(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Null)
}
fn false_fn(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(false))
}
fn true_fn(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Bool(true))
}
fn zero_fn(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Num(0.0))
}
fn ignore_fn(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Undefined)
}
fn empty_string(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js(""))
}
fn empty_array(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(vm.new_array(Vec::new())))
}

/// A promise already rejected (requestFullscreen and the like, which this browser doesn't do).
fn rejected_fn(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    let e = dom_exception(vm, "NotSupportedError", "Not supported here.");
    vm.reject_promise(p, e);
    if let Class::Promise(d) = &mut vm.heap.get_mut(p).class {
        d.handled = true;
    }
    Ok(Value::Obj(p))
}

pub fn doc(vm: &Vm) -> &Document {
    &host_ref(vm).doc
}

fn element_of<'a>(vm: &'a Vm, n: NodeId) -> Option<&'a Element> {
    host_ref(vm).doc.element(n)
}

fn node_list(vm: &mut Vm, ids: Vec<NodeId>) -> Value {
    let items: Vec<Value> = ids.into_iter().map(|n| wrap(vm, n)).collect();
    let a = vm.new_array(items);
    if let Some(p) = host_ref(vm).proto("NodeList") {
        vm.heap.get_mut(a).proto = Some(p);
    }
    Value::Obj(a)
}

// --------------------------------------------------------------------------------------------- Node

fn node_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(Value::Num(match &doc(vm).nodes[n].data {
        Data::Element(_) => 1.0,
        Data::Text(_) => 3.0,
        Data::Comment(_) => 8.0,
        Data::Document => 9.0,
        Data::Fragment => 11.0,
    }))
}

fn node_name(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(js(&match &doc(vm).nodes[n].data {
        Data::Element(e) => html_upper(doc(vm), n, &e.name),
        Data::Text(_) => String::from("#text"),
        Data::Comment(_) => String::from("#comment"),
        Data::Document => String::from("#document"),
        Data::Fragment => String::from("#document-fragment"),
    }))
}

/// An element's tagName: upper case for HTML's, as it is for SVG's.
fn html_upper(doc: &Document, n: NodeId, name: &str) -> String {
    let mut at = Some(n);
    while let Some(p) = at {
        if doc.name(p) == "svg" {
            return String::from(name);
        }
        at = doc.parent(p);
    }
    name.to_ascii_uppercase()
}

fn node_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(match doc(vm).data_of(n) {
        Some(t) => js(t),
        None => Value::Null,
    })
}

fn set_node_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let v = arg(args, 0);
    let s = if matches!(v, Value::Null) { String::new() } else { string(vm, &v)? };
    set_data(vm, n, s);
    Ok(Value::Undefined)
}

/// A text's or comment's words changed.
fn set_data(vm: &mut Vm, n: NodeId, s: String) {
    let old = {
        let h = host(vm);
        let old = match &mut h.doc.nodes[n].data {
            Data::Text(t) | Data::Comment(t) => Some(core::mem::replace(t, s)),
            _ => None,
        };
        if old.is_some() {
            h.changed();
        }
        old
    };
    if let Some(o) = old {
        record(vm, Change::Data(n, o));
    }
}

fn text_content(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let d = doc(vm);
    Ok(match &d.nodes[n].data {
        Data::Document => Value::Null,
        Data::Text(t) | Data::Comment(t) => js(t),
        _ => js(&d.text_of(n)),
    })
}

fn set_text_content(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let v = arg(args, 0);
    let s = if v.is_nullish() { String::new() } else { string(vm, &v)? };
    match &doc(vm).nodes[n].data {
        Data::Text(_) | Data::Comment(_) => set_data(vm, n, s),
        Data::Document => {}
        _ => {
            let node = if s.is_empty() { None } else { Some(host(vm).doc.create(Data::Text(s))) };
            replace_all(vm, n, node);
        }
    }
    Ok(Value::Undefined)
}

/// All of a node's children replaced by one node (or none).
fn replace_all(vm: &mut Vm, parent: NodeId, node: Option<NodeId>) {
    let parent = content_of(vm, parent);
    let old: Vec<NodeId> = doc(vm).children(parent).to_vec();
    for &c in &old {
        host(vm).doc.detach(c);
        removed(vm, c);
    }
    let mut added = Vec::new();
    if let Some(n) = node {
        let kids = if matches!(doc(vm).nodes[n].data, Data::Fragment) { doc(vm).children(n).to_vec() } else { vec![n] };
        for k in &kids {
            host(vm).doc.insert(parent, *k, None);
        }
        added = kids;
    }
    host(vm).changed();
    if !old.is_empty() || !added.is_empty() {
        record(vm, Change::Children(parent, added.clone(), old, None, None));
    }
    for a in added {
        inserted(vm, a);
    }
}

/// A <template>'s contents (its children's real place), or the node itself.
fn content_of(vm: &mut Vm, n: NodeId) -> NodeId {
    if doc(vm).name(n) == "template" { host(vm).doc.template_content(n) } else { n }
}

fn parent_node(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let p = doc(vm).parent(n);
    Ok(wrap_opt(vm, p))
}

fn parent_element(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let p = doc(vm).parent_element(n);
    Ok(wrap_opt(vm, p))
}

fn first_child(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let c = doc(vm).children(n).first().copied();
    Ok(wrap_opt(vm, c))
}

fn last_child(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let c = doc(vm).children(n).last().copied();
    Ok(wrap_opt(vm, c))
}

fn previous_sibling(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let c = doc(vm).previous_sibling(n);
    Ok(wrap_opt(vm, c))
}

fn next_sibling(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let c = doc(vm).next_sibling(n);
    Ok(wrap_opt(vm, c))
}

fn owner_document(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    if n == 0 {
        return Ok(Value::Null);
    }
    Ok(wrap(vm, 0))
}

fn is_connected(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(Value::Bool(doc(vm).connected(n)))
}

fn base_uri(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    this_node(vm, this)?;
    Ok(js(&host_ref(vm).base.href()))
}

fn child_nodes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(super::lists::live(vm, n, ListKind::ChildNodes))
}

/// Whether a node can hold children.
fn can_parent(d: &Document, n: NodeId) -> bool {
    matches!(d.nodes[n].data, Data::Document | Data::Element(_) | Data::Fragment)
}

/// Inserts `node` (a fragment's children, for a fragment) into `parent` before `before`, checking it may.
pub fn pre_insert(vm: &mut Vm, parent: NodeId, node: NodeId, before: Option<NodeId>) -> Result<(), Value> {
    let d = doc(vm);
    if !can_parent(d, parent) {
        return Err(dom_exception(vm, "HierarchyRequestError", "This node type does not support this method."));
    }
    if d.contains(node, parent) {
        return Err(dom_exception(vm, "HierarchyRequestError", "The new child element contains the parent."));
    }
    if let Some(b) = before
        && d.parent(b) != Some(parent)
    {
        return Err(dom_exception(vm, "NotFoundError", "The node before which the new node is to be inserted is not a child of this node."));
    }
    if matches!(d.nodes[node].data, Data::Document) {
        return Err(dom_exception(vm, "HierarchyRequestError", "Nodes of type '#document' may not be inserted."));
    }
    let parent = content_of(vm, parent);
    // (Inserting a node before itself: before its next sibling then.)
    let before = if before == Some(node) { doc(vm).next_sibling(node) } else { before };
    let kids: Vec<NodeId> = if matches!(doc(vm).nodes[node].data, Data::Fragment) { doc(vm).children(node).to_vec() } else { vec![node] };
    if kids.is_empty() {
        return Ok(());
    }
    // Where it was, for observers.
    let old_parent = if kids.len() == 1 { doc(vm).parent(node) } else { Some(node) };
    if let Some(op) = old_parent {
        let mut removed_from: Vec<NodeId> = Vec::new();
        for &k in &kids {
            if doc(vm).parent(k) == Some(op) {
                removed_from.push(k);
            }
        }
        if !removed_from.is_empty() {
            let prev = doc(vm).previous_sibling(removed_from[0]);
            let next = doc(vm).next_sibling(*removed_from.last().unwrap_or(&node));
            for &k in &removed_from {
                host(vm).doc.detach(k);
            }
            record(vm, Change::Children(op, Vec::new(), removed_from, prev, next));
        }
    }
    let prev = match before {
        Some(b) => doc(vm).previous_sibling(b),
        None => doc(vm).children(parent).last().copied(),
    };
    for &k in &kids {
        host(vm).doc.insert(parent, k, before);
    }
    host(vm).changed();
    record(vm, Change::Children(parent, kids.clone(), Vec::new(), prev, before));
    for k in kids {
        inserted(vm, k);
    }
    Ok(())
}

/// A node taken out of its parent (by a script).
pub fn remove(vm: &mut Vm, n: NodeId) {
    let Some(p) = doc(vm).parent(n) else { return };
    let prev = doc(vm).previous_sibling(n);
    let next = doc(vm).next_sibling(n);
    let was_connected = doc(vm).connected(n);
    host(vm).doc.detach(n);
    host(vm).changed();
    if host_ref(vm).focus.is_some_and(|f| doc(vm).contains(n, f) || f == n) {
        host(vm).focus = None;
    }
    record(vm, Change::Children(p, Vec::new(), vec![n], prev, next));
    if was_connected {
        removed(vm, n);
    }
}

/// What a node put in the page brings: its scripts run (inline) or fetched, its custom elements upgraded and told,
/// its style sheets and pictures read again.
fn inserted(vm: &mut Vm, n: NodeId) {
    if !doc(vm).connected(n) {
        return;
    }
    let all: Vec<NodeId> = doc(vm).descendants(n).collect();
    for id in all {
        let name = String::from(doc(vm).name(id));
        match name.as_str() {
            "script" => script_inserted(vm, id),
            "link" | "style" | "img" | "picture" | "source" | "iframe" | "base" | "canvas" | "svg" | "video" => host(vm).rescan = true,
            "input" if attr(vm, id, "type").is_some_and(|t| t.eq_ignore_ascii_case("image")) => host(vm).rescan = true,
            "title" => host(vm).title_changed = true,
            _ => {}
        }
        if name.contains('-') {
            upgrade(vm, id);
            callback(vm, id, "connectedCallback", &[]);
        }
    }
}

/// A node taken out of the page: its custom elements told.
fn removed(vm: &mut Vm, n: NodeId) {
    if host_ref(vm).custom.is_empty() {
        return;
    }
    let all: Vec<NodeId> = doc(vm).descendants(n).filter(|&id| doc(vm).name(id).contains('-')).collect();
    for id in all {
        callback(vm, id, "disconnectedCallback", &[]);
    }
}

/// A <script> put in the page by a script: one with text runs now, one with an address is fetched (and runs when
/// it comes).
fn script_inserted(vm: &mut Vm, id: NodeId) {
    if host_ref(vm).started.contains(&id) {
        return;
    }
    let (src, ty, nomodule) = {
        let e = element_of(vm, id);
        (e.and_then(|e| e.attr("src")).map(String::from), e.and_then(|e| e.attr("type")).map(String::from), e.is_some_and(|e| e.has_attr("nomodule")))
    };
    let kind = script_kind(ty.as_deref());
    if kind == ScriptKind::Data || (kind == ScriptKind::Classic && nomodule) {
        return;
    }
    if kind == ScriptKind::ImportMap {
        let text = super::text_of(doc(vm), id);
        if !text.trim().is_empty() {
            host(vm).started.push(id);
            host(vm).new_scripts.push(id);
        }
        return;
    }
    // (A module, inline or not, is loaded by the page and runs once its modules have come.)
    if kind == ScriptKind::Module && src.is_none() && super::text_of(doc(vm), id).trim().is_empty() {
        return;
    }
    host(vm).started.push(id);
    if src.is_some() || kind == ScriptKind::Module {
        host(vm).new_scripts.push(id);
        host(vm).rescan = true;
        return;
    }
    let code = super::text_of(doc(vm), id);
    if code.trim().is_empty() {
        // (An empty one runs when it gets text.)
        host(vm).started.retain(|&s| s != id);
        return;
    }
    let saved = host_ref(vm).current_script;
    host(vm).current_script = Some(id);
    let r = vm.run_script(&code, "inline script");
    host(vm).current_script = saved;
    if let Err(e) = r {
        super::report(vm, &e, "an inline script");
    }
}

/// What a <script> is, by its type attribute.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScriptKind {
    Classic,
    Module,
    ImportMap,
    /// Not a script (a template's text, JSON data...).
    Data,
}

pub fn script_kind(ty: Option<&str>) -> ScriptKind {
    if runnable_type(ty) {
        return ScriptKind::Classic;
    }
    match ty.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        Some("module") => ScriptKind::Module,
        Some("importmap") => ScriptKind::ImportMap,
        _ => ScriptKind::Data,
    }
}

/// Whether a script's type attribute is JavaScript's (classic scripts: no type, or a JavaScript one).
pub fn runnable_type(ty: Option<&str>) -> bool {
    match ty.map(|t| t.trim().to_ascii_lowercase()) {
        None => true,
        Some(t) => t.is_empty() || matches!(t.as_str(), "text/javascript" | "application/javascript" | "text/ecmascript" | "application/ecmascript" | "application/x-javascript" | "text/jscript" | "text/livescript" | "text/x-javascript" | "text/x-ecmascript" | "text/javascript1.0" | "text/javascript1.1" | "text/javascript1.2" | "text/javascript1.3" | "text/javascript1.4" | "text/javascript1.5"),
    }
}

fn append_child(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    let c = node_arg(vm, args, 0, "appendChild")?;
    pre_insert(vm, p, c, None)?;
    Ok(arg(args, 0))
}

fn insert_before(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    let c = node_arg(vm, args, 0, "insertBefore")?;
    let before = match arg(args, 1) {
        Value::Null | Value::Undefined => None,
        v => Some(node_of(vm, &v).ok_or_else(|| vm.type_error("Failed to execute 'insertBefore' on 'Node': parameter 2 is not of type 'Node'."))?),
    };
    pre_insert(vm, p, c, before)?;
    Ok(arg(args, 0))
}

fn remove_child(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    let c = node_arg(vm, args, 0, "removeChild")?;
    let p = if doc(vm).name(p) == "template" { content_of(vm, p) } else { p };
    if doc(vm).parent(c) != Some(p) {
        return Err(dom_exception(vm, "NotFoundError", "The node to be removed is not a child of this node."));
    }
    remove(vm, c);
    Ok(arg(args, 0))
}

fn replace_child(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    let new = node_arg(vm, args, 0, "replaceChild")?;
    let old = node_arg(vm, args, 1, "replaceChild")?;
    if doc(vm).parent(old) != Some(p) {
        return Err(dom_exception(vm, "NotFoundError", "The node to be replaced is not a child of this node."));
    }
    if new == old {
        return Ok(arg(args, 1));
    }
    let next = doc(vm).next_sibling(old);
    let next = if next == Some(new) { doc(vm).next_sibling(new) } else { next };
    remove(vm, old);
    pre_insert(vm, p, new, next)?;
    Ok(arg(args, 1))
}

fn clone_node(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    if n == 0 {
        return Err(dom_exception(vm, "NotSupportedError", "Cloning the document isn't supported."));
    }
    let deep = arg(args, 0).truthy();
    let c = host(vm).doc.clone_node(n, deep);
    let w = wrap(vm, c);
    if doc(vm).name(c).contains('-') {
        upgrade(vm, c);
    }
    Ok(w)
}

fn contains(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(Value::Bool(match node_of(vm, &arg(args, 0)) {
        Some(o) => doc(vm).contains(n, o),
        None => false,
    }))
}

fn has_child_nodes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(Value::Bool(!doc(vm).children(n).is_empty()))
}

fn get_root_node(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let r = doc(vm).root_of(n);
    Ok(wrap(vm, r))
}

fn compare_position(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let a = this_node(vm, this)?;
    let b = node_arg(vm, args, 0, "compareDocumentPosition")?;
    if a == b {
        return Ok(Value::Num(0.0));
    }
    let d = doc(vm);
    if d.root_of(a) != d.root_of(b) {
        return Ok(Value::Num(1.0 + 32.0 + if a < b { 4.0 } else { 2.0 }));
    }
    if d.contains(a, b) {
        return Ok(Value::Num(16.0 + 4.0));
    }
    if d.contains(b, a) {
        return Ok(Value::Num(8.0 + 2.0));
    }
    // Which comes first in the tree's order.
    let root = d.root_of(a);
    for id in d.descendants(root) {
        if id == a {
            return Ok(Value::Num(4.0));
        }
        if id == b {
            return Ok(Value::Num(2.0));
        }
    }
    Ok(Value::Num(1.0))
}

fn is_same_node(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(Value::Bool(node_of(vm, &arg(args, 0)) == Some(n)))
}

fn is_equal_node(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let Some(o) = node_of(vm, &arg(args, 0)) else { return Ok(Value::Bool(false)) };
    let d = doc(vm);
    Ok(Value::Bool(d.outer_html(n) == d.outer_html(o)))
}

fn normalize(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let all: Vec<NodeId> = doc(vm).descendants(n).collect();
    for p in all {
        let kids = doc(vm).children(p).to_vec();
        let mut last_text: Option<NodeId> = None;
        for k in kids {
            match &doc(vm).nodes[k].data {
                Data::Text(t) if t.is_empty() => remove(vm, k),
                Data::Text(t) => {
                    if let Some(l) = last_text {
                        let t = t.clone();
                        if let Data::Text(lt) = &mut host(vm).doc.nodes[l].data {
                            lt.push_str(&t);
                        }
                        remove(vm, k);
                    } else {
                        last_text = Some(k);
                    }
                }
                _ => last_text = None,
            }
        }
    }
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------- CharacterData, Text

fn this_data(vm: &mut Vm, this: &Value) -> Result<(NodeId, Vec<u16>), Value> {
    let n = this_node(vm, this)?;
    match doc(vm).data_of(n) {
        Some(t) => Ok((n, t.encode_utf16().collect())),
        None => Err(vm.type_error("Illegal invocation")),
    }
}

fn data_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, u) = this_data(vm, this)?;
    Ok(Value::Num(u.len() as f64))
}

/// Offsets in UTF-16 units, as scripts count.
fn units_to_string(u: &[u16]) -> String {
    String::from_utf16_lossy(u)
}

fn edit_data(vm: &mut Vm, this: &Value, at: &Value, count: &Value, insert: &str) -> Result<String, Value> {
    let (n, mut u) = this_data(vm, this)?;
    let at = vm.to_uint32(at)? as usize;
    if at > u.len() {
        return Err(dom_exception(vm, "IndexSizeError", "The offset is greater than the node's length."));
    }
    let count = if count.is_undefined() { 0 } else { vm.to_uint32(count)? as usize };
    let end = (at + count).min(u.len());
    let removed: Vec<u16> = u.splice(at..end, insert.encode_utf16()).collect();
    set_data(vm, n, units_to_string(&u));
    Ok(units_to_string(&removed))
}

fn append_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, u) = this_data(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    let mut t = units_to_string(&u);
    t.push_str(&s);
    set_data(vm, n, t);
    Ok(Value::Undefined)
}

fn insert_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 1))?;
    edit_data(vm, this, &arg(args, 0), &Value::Num(0.0), &s)?;
    Ok(Value::Undefined)
}

fn delete_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    edit_data(vm, this, &arg(args, 0), &arg(args, 1), "")?;
    Ok(Value::Undefined)
}

fn replace_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 2))?;
    edit_data(vm, this, &arg(args, 0), &arg(args, 1), &s)?;
    Ok(Value::Undefined)
}

fn substring_data(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (_, u) = this_data(vm, this)?;
    let at = vm.to_uint32(&arg(args, 0))? as usize;
    if at > u.len() {
        return Err(dom_exception(vm, "IndexSizeError", "The offset is greater than the node's length."));
    }
    let count = vm.to_uint32(&arg(args, 1))? as usize;
    Ok(js(&units_to_string(&u[at..(at + count).min(u.len())])))
}

fn whole_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, _) = this_data(vm, this)?;
    let d = doc(vm);
    // The texts next to it, all together.
    let mut first = n;
    while let Some(p) = d.previous_sibling(first) {
        if matches!(d.nodes[p].data, Data::Text(_)) { first = p } else { break }
    }
    let mut out = String::new();
    let mut at = Some(first);
    while let Some(k) = at {
        match &d.nodes[k].data {
            Data::Text(t) => out.push_str(t),
            _ => break,
        }
        at = d.next_sibling(k);
    }
    Ok(js(&out))
}

fn split_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, u) = this_data(vm, this)?;
    let at = vm.to_uint32(&arg(args, 0))? as usize;
    if at > u.len() {
        return Err(dom_exception(vm, "IndexSizeError", "The offset is greater than the node's length."));
    }
    let rest = units_to_string(&u[at..]);
    set_data(vm, n, units_to_string(&u[..at]));
    let new = host(vm).doc.create(Data::Text(rest));
    if let Some(p) = doc(vm).parent(n) {
        let next = doc(vm).next_sibling(n);
        pre_insert(vm, p, new, next)?;
    }
    Ok(wrap(vm, new))
}

fn text_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let s = if args.is_empty() { String::new() } else { string(vm, &args[0])? };
    let n = host(vm).doc.create(Data::Text(s));
    let w = wrap(vm, n);
    retarget_proto(vm, &w, nt, "Text")?;
    Ok(w)
}

fn comment_new(vm: &mut Vm, nt: &Value, args: &[Value], _f: Gc) -> R {
    let s = if args.is_empty() { String::new() } else { string(vm, &args[0])? };
    let n = host(vm).doc.create(Data::Comment(s));
    let w = wrap(vm, n);
    retarget_proto(vm, &w, nt, "Comment")?;
    Ok(w)
}

fn fragment_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let n = host(vm).doc.create(Data::Fragment);
    let w = wrap(vm, n);
    retarget_proto(vm, &w, nt, "DocumentFragment")?;
    Ok(w)
}

fn document_new(vm: &mut Vm, _nt: &Value, _args: &[Value], _f: Gc) -> R {
    // (A new document: a fragment of this one stands in for it.)
    let n = host(vm).doc.create(Data::Fragment);
    Ok(wrap(vm, n))
}

/// A subclass's object gets its prototype (new.target's).
fn retarget_proto(vm: &mut Vm, w: &Value, nt: &Value, iface: &str) -> Result<(), Value> {
    let p = proto_from(vm, nt, iface)?;
    if let Value::Obj(g) = w {
        vm.heap.get_mut(*g).proto = Some(p);
    }
    Ok(())
}

fn doctype_part(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    Ok(match vm.data(f) {
        Value::Str(s) if s.to_rust() == "name" => js("html"),
        _ => js(""),
    })
}

// -------------------------------------------------------------------------------- ParentNode, ChildNode

fn parent_node_methods(vm: &mut Vm, p: Gc) {
    attribute(vm, p, "children", children, None);
    attribute(vm, p, "firstElementChild", first_element_child, None);
    attribute(vm, p, "lastElementChild", last_element_child, None);
    attribute(vm, p, "childElementCount", child_element_count, None);
    method(vm, p, "append", 0, append);
    method(vm, p, "prepend", 0, prepend);
    method(vm, p, "replaceChildren", 0, replace_children);
    method(vm, p, "querySelector", 1, query_selector);
    method(vm, p, "querySelectorAll", 1, query_selector_all);
}

fn child_node_methods(vm: &mut Vm, p: Gc) {
    method(vm, p, "before", 0, before);
    method(vm, p, "after", 0, after);
    method(vm, p, "replaceWith", 0, replace_with);
    method(vm, p, "remove", 0, remove_self);
}

fn sibling_element_getters(vm: &mut Vm, p: Gc) {
    attribute(vm, p, "previousElementSibling", previous_element_sibling, None);
    attribute(vm, p, "nextElementSibling", next_element_sibling, None);
}

fn children(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(super::lists::live(vm, n, ListKind::Children))
}

fn element_children(d: &Document, n: NodeId) -> Vec<NodeId> {
    d.children(n).iter().copied().filter(|&c| d.element(c).is_some()).collect()
}

fn first_element_child(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let c = element_children(doc(vm), n).first().copied();
    Ok(wrap_opt(vm, c))
}

fn last_element_child(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let c = element_children(doc(vm), n).last().copied();
    Ok(wrap_opt(vm, c))
}

fn child_element_count(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(Value::Num(element_children(doc(vm), n).len() as f64))
}

fn previous_element_sibling(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let d = doc(vm);
    let mut at = d.previous_sibling(n);
    while let Some(s) = at {
        if d.element(s).is_some() {
            break;
        }
        at = d.previous_sibling(s);
    }
    Ok(wrap_opt(vm, at))
}

fn next_element_sibling(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let d = doc(vm);
    let mut at = d.next_sibling(n);
    while let Some(s) = at {
        if d.element(s).is_some() {
            break;
        }
        at = d.next_sibling(s);
    }
    Ok(wrap_opt(vm, at))
}

/// append()'s and the like's arguments as one node: nodes as they are, anything else as text (a fragment holds
/// them if there are several).
fn nodes_from(vm: &mut Vm, args: &[Value]) -> Result<Option<NodeId>, Value> {
    let mut ids = Vec::new();
    for a in args {
        match node_of(vm, a) {
            Some(n) => ids.push(n),
            None => {
                let s = string(vm, a)?;
                ids.push(host(vm).doc.create(Data::Text(s)));
            }
        }
    }
    match ids.len() {
        0 => Ok(None),
        1 => Ok(Some(ids[0])),
        _ => {
            let f = host(vm).doc.create(Data::Fragment);
            for i in ids {
                host(vm).doc.insert(f, i, None);
            }
            Ok(Some(f))
        }
    }
}

fn append(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    if let Some(n) = nodes_from(vm, args)? {
        pre_insert(vm, p, n, None)?;
    }
    Ok(Value::Undefined)
}

fn prepend(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    if let Some(n) = nodes_from(vm, args)? {
        let first = doc(vm).children(p).first().copied();
        pre_insert(vm, p, n, first)?;
    }
    Ok(Value::Undefined)
}

fn replace_children(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let p = this_node(vm, this)?;
    let n = nodes_from(vm, args)?;
    replace_all(vm, p, n);
    Ok(Value::Undefined)
}

fn before(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let Some(p) = doc(vm).parent(n) else { return Ok(Value::Undefined) };
    if let Some(x) = nodes_from(vm, args)? {
        pre_insert(vm, p, x, Some(n))?;
    }
    Ok(Value::Undefined)
}

fn after(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let Some(p) = doc(vm).parent(n) else { return Ok(Value::Undefined) };
    if let Some(x) = nodes_from(vm, args)? {
        let next = doc(vm).next_sibling(n);
        pre_insert(vm, p, x, next)?;
    }
    Ok(Value::Undefined)
}

fn replace_with(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let Some(p) = doc(vm).parent(n) else { return Ok(Value::Undefined) };
    let next = doc(vm).next_sibling(n);
    let x = nodes_from(vm, args)?;
    remove(vm, n);
    if let Some(x) = x {
        let next = if next.is_some_and(|nx| doc(vm).parent(nx) == Some(p)) { next } else { None };
        pre_insert(vm, p, x, next)?;
    }
    Ok(Value::Undefined)
}

fn remove_self(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    remove(vm, n);
    Ok(Value::Undefined)
}

// ------------------------------------------------------------------------------------------ selectors

/// The elements inside `root` (not it) a selector matches, in the tree's order (the first only, if `first`).
pub fn select(vm: &mut Vm, root: NodeId, text: &str, first: bool) -> Result<Vec<NodeId>, Value> {
    let sels = parse_selector(vm, text)?;
    let d = doc(vm);
    let sib = style::SiblingInfo::new(d);
    let mut out = Vec::new();
    for id in d.descendants(root).skip(1) {
        if d.element(id).is_some() && sels.iter().any(|s| style::matches(d, &sib, id, s)) {
            out.push(id);
            if first {
                break;
            }
        }
    }
    Ok(out)
}

fn parse_selector(vm: &mut Vm, text: &str) -> Result<Vec<css::Selector>, Value> {
    // (:scope is the element itself, at the start: what's inside it.)
    let t = text.trim();
    let t = t.strip_prefix(":scope").map(|r| r.trim_start()).unwrap_or(t);
    let t = t.strip_prefix('>').map(|r| r.trim_start()).unwrap_or(t);
    match css::selector_list(&css::tokens(t)) {
        Some(s) if !t.is_empty() => Ok(s),
        _ => Err(dom_exception(vm, "SyntaxError", &format!("'{text}' is not a valid selector."))),
    }
}

fn query_selector(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let s = str_arg(vm, args, 0, "querySelector")?;
    let found = select(vm, n, &s, true)?;
    Ok(wrap_opt(vm, found.first().copied()))
}

fn query_selector_all(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let s = str_arg(vm, args, 0, "querySelectorAll")?;
    let found = select(vm, n, &s, false)?;
    Ok(node_list(vm, found))
}

fn matches(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let s = str_arg(vm, args, 0, "matches")?;
    let sels = parse_selector(vm, &s)?;
    let d = doc(vm);
    let sib = style::SiblingInfo::new(d);
    Ok(Value::Bool(sels.iter().any(|sel| style::matches(d, &sib, n, sel))))
}

fn closest(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let s = str_arg(vm, args, 0, "closest")?;
    let sels = parse_selector(vm, &s)?;
    let d = doc(vm);
    let sib = style::SiblingInfo::new(d);
    let mut at = Some(n);
    while let Some(e) = at {
        if d.element(e).is_some() && sels.iter().any(|sel| style::matches(d, &sib, e, sel)) {
            return Ok(wrap(vm, e));
        }
        at = d.parent_element(e);
    }
    Ok(Value::Null)
}

fn get_elements_by_tag_name(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let name = str_arg(vm, args, 0, "getElementsByTagName")?.to_ascii_lowercase();
    Ok(super::lists::live(vm, n, ListKind::Tag(name)))
}

fn get_elements_by_tag_name_ns(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    get_elements_by_tag_name(vm, this, &[arg(args, 1)], _f)
}

fn get_elements_by_class_name(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let names = str_arg(vm, args, 0, "getElementsByClassName")?;
    let classes: Vec<String> = names.split_ascii_whitespace().map(String::from).collect();
    Ok(super::lists::live(vm, n, ListKind::Class(classes)))
}

fn get_element_by_id(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let id = str_arg(vm, args, 0, "getElementById")?;
    let d = doc(vm);
    let found = d.descendants(n).find(|&e| d.element(e).and_then(|e| e.attr("id")) == Some(id.as_str()));
    Ok(wrap_opt(vm, found))
}

// --------------------------------------------------------------------------------------------- Element

fn tag_name(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = String::from(doc(vm).name(n));
    Ok(js(&html_upper(doc(vm), n, &name)))
}

fn local_name(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(doc(vm).name(n)))
}

fn namespace_uri(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let mut at = Some(n);
    while let Some(p) = at {
        match d.name(p) {
            "svg" => return Ok(js("http://www.w3.org/2000/svg")),
            "math" => return Ok(js("http://www.w3.org/1998/Math/MathML")),
            _ => {}
        }
        at = d.parent(p);
    }
    Ok(js("http://www.w3.org/1999/xhtml"))
}

/// An attribute's value, if the element has it.
pub fn attr(vm: &Vm, n: NodeId, name: &str) -> Option<String> {
    element_of(vm, n).and_then(|e| e.attr(name)).map(String::from)
}

/// Sets an attribute (as a script does): observers told, custom elements' callbacks called, the page's view of it
/// worked out again.
pub fn set_attr(vm: &mut Vm, n: NodeId, name: &str, value: &str) {
    let old = attr(vm, n, name);
    if old.as_deref() == Some(value) && !matches!(name, "src" | "href") {
        return;
    }
    if let Some(e) = host(vm).doc.element_mut(n) {
        e.set_attr(name, value);
    }
    attr_changed(vm, n, name, old, Some(String::from(value)));
}

pub fn remove_attr(vm: &mut Vm, n: NodeId, name: &str) {
    let old = attr(vm, n, name);
    if old.is_none() {
        return;
    }
    if let Some(e) = host(vm).doc.element_mut(n) {
        e.remove_attr(name);
    }
    attr_changed(vm, n, name, old, None);
}

fn attr_changed(vm: &mut Vm, n: NodeId, name: &str, old: Option<String>, new: Option<String>) {
    host(vm).changed();
    record(vm, Change::Attribute(n, name, old.clone()));
    let el = String::from(doc(vm).name(n));
    if doc(vm).connected(n) && (matches!((el.as_str(), name), ("link", "href" | "rel" | "media") | ("img", "src" | "srcset") | ("source", "src" | "srcset") | ("base", "href") | ("canvas", "width" | "height"))) {
        host(vm).rescan = true;
    }
    if el == "canvas" && matches!(name, "width" | "height") {
        super::canvas::resized(vm, n);
    }
    if el == "img" && matches!(name, "src" | "srcset") {
        // (A new picture: not complete until it comes; one not in the page fetched all the same.)
        let h = host(vm);
        h.settled.remove(&n);
        if !h.doc.connected(n) {
            if !h.images.contains(&n) {
                h.images.push(n);
            }
            h.rescan = true;
        }
    }
    if el == "script" && name == "src" && doc(vm).connected(n) {
        script_inserted(vm, n);
    }
    if el.contains('-') && observed(vm, &el, name) {
        let args = [js(name), old.map_or(Value::Null, |o| js(&o)), new.map_or(Value::Null, |v| js(&v)), Value::Null];
        callback(vm, n, "attributeChangedCallback", &args);
    }
}

fn attr_name_arg(vm: &mut Vm, args: &[Value], i: usize, what: &str) -> Result<String, Value> {
    let s = str_arg(vm, args, i, what)?;
    // (HTML's attribute names are lower case.)
    Ok(s.to_ascii_lowercase())
}

fn get_attribute(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = attr_name_arg(vm, args, 0, "getAttribute")?;
    Ok(attr(vm, n, &name).map_or(Value::Null, |v| js(&v)))
}

fn get_attribute_ns(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    get_attribute(vm, this, &[arg(args, 1)], f)
}

fn set_attribute(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = attr_name_arg(vm, args, 0, "setAttribute")?;
    if name.is_empty() || name.contains(|c: char| c.is_ascii_whitespace() || matches!(c, '"' | '\'' | '>' | '/' | '=' | '<')) {
        return Err(dom_exception(vm, "InvalidCharacterError", &format!("'{name}' is not a valid attribute name.")));
    }
    let v = string(vm, &arg(args, 1))?;
    set_attr(vm, n, &name, &v);
    Ok(Value::Undefined)
}

fn set_attribute_ns(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let name = str_arg(vm, args, 1, "setAttributeNS")?;
    let local = String::from(name.rsplit(':').next().unwrap_or(&name));
    set_attribute(vm, this, &[js(&local), arg(args, 2)], f)
}

fn remove_attribute(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = attr_name_arg(vm, args, 0, "removeAttribute")?;
    remove_attr(vm, n, &name);
    Ok(Value::Undefined)
}

fn remove_attribute_ns(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    remove_attribute(vm, this, &[arg(args, 1)], f)
}

fn has_attribute(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = attr_name_arg(vm, args, 0, "hasAttribute")?;
    Ok(Value::Bool(attr(vm, n, &name).is_some()))
}

fn has_attribute_ns(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    has_attribute(vm, this, &[arg(args, 1)], f)
}

fn has_attributes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Bool(element_of(vm, n).is_some_and(|e| !e.attrs.is_empty())))
}

fn toggle_attribute(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = attr_name_arg(vm, args, 0, "toggleAttribute")?;
    let has = attr(vm, n, &name).is_some();
    let want = if args.len() > 1 { args[1].truthy() } else { !has };
    if want && !has {
        set_attr(vm, n, &name, "");
    } else if !want && has {
        remove_attr(vm, n, &name);
    }
    Ok(Value::Bool(want))
}

fn get_attribute_names(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let names: Vec<Value> = element_of(vm, n).map(|e| e.attrs.iter().map(|(k, _)| js(k)).collect()).unwrap_or_default();
    Ok(Value::Obj(vm.new_array(names)))
}

/// An Attr object for an element's attribute (its element and name kept on it).
fn attr_node(vm: &mut Vm, n: NodeId, name: &str) -> Value {
    let p = host_ref(vm).proto("Attr").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Ordinary);
    let w = wrap(vm, n);
    set_slot(vm, g, host_ref(vm).syms.target, w);
    set_slot(vm, g, host_ref(vm).syms.data, js(name));
    Value::Obj(g)
}

fn this_attr(vm: &mut Vm, this: &Value) -> Result<(NodeId, String), Value> {
    let t = slot(vm, this, host_ref(vm).syms.target);
    let name = slot(vm, this, host_ref(vm).syms.data);
    match (node_of(vm, &t), name) {
        (Some(n), Value::Str(s)) => Ok((n, s.to_rust())),
        _ => Err(vm.type_error("Illegal invocation")),
    }
}

fn attr_name(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (_, name) = this_attr(vm, this)?;
    Ok(js(&name))
}

fn attr_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, name) = this_attr(vm, this)?;
    Ok(js(&attr(vm, n, &name).unwrap_or_default()))
}

fn set_attr_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let (n, name) = this_attr(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, &name, &v);
    Ok(Value::Undefined)
}

fn attr_owner(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (n, _) = this_attr(vm, this)?;
    Ok(wrap(vm, n))
}

fn get_attribute_node(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let name = attr_name_arg(vm, args, 0, "getAttributeNode")?;
    if attr(vm, n, &name).is_none() {
        return Ok(Value::Null);
    }
    Ok(attr_node(vm, n, &name))
}

/// element.attributes: an array-like of its attributes (as they are now), by place and by name.
fn attributes(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let names: Vec<String> = element_of(vm, n).map(|e| e.attrs.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default();
    let mut items = Vec::new();
    for name in &names {
        items.push(attr_node(vm, n, name));
    }
    let a = vm.new_array(items.clone());
    for (name, item) in names.iter().zip(items) {
        if name.parse::<u32>().is_err() && name != "length" {
            vm.put(a, name, item, 0);
        }
    }
    if let Some(p) = host_ref(vm).proto("NamedNodeMap") {
        vm.heap.get_mut(a).proto = Some(p);
    }
    Ok(Value::Obj(a))
}

fn inner_html(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(js(&doc(vm).inner_html(n)))
}

/// HTML read as the insides of an element named `context`: a fragment of this page's (in no tree yet).
pub fn fragment(vm: &mut Vm, html: &str, context: &str) -> NodeId {
    let parsed = crate::html::parse_fragment(html, context, true);
    let f = host(vm).doc.create(Data::Fragment);
    let kids: Vec<NodeId> = parsed.children(1).to_vec();
    for k in kids {
        let c = import(&mut host(vm).doc, &parsed, k);
        host(vm).doc.insert(f, c, None);
    }
    f
}

/// A node (and what's inside it) copied from another document into this one.
pub fn import(to: &mut Document, from: &Document, id: NodeId) -> NodeId {
    let copy = |d: &Data| match d {
        Data::Element(e) => Data::Element(Element { name: e.name.clone(), attrs: e.attrs.clone() }),
        Data::Text(t) => Data::Text(t.clone()),
        Data::Comment(t) => Data::Comment(t.clone()),
        _ => Data::Fragment,
    };
    let root = to.create(copy(&from.nodes[id].data));
    let mut work = vec![(id, root)];
    while let Some((src, dst)) = work.pop() {
        for &k in from.children(src) {
            let c = to.create(copy(&from.nodes[k].data));
            to.nodes[c].parent = Some(dst);
            to.nodes[dst].children.push(c);
            work.push((k, c));
        }
        if let Some(&(_, f)) = from.templates.iter().find(|(t, _)| *t == src) {
            let fc = import(to, from, f);
            to.templates.push((dst, fc));
        }
    }
    root
}

fn set_inner_html(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    let v = arg(args, 0);
    let html = if matches!(v, Value::Null) { String::new() } else { string(vm, &v)? };
    let context = String::from(doc(vm).name(n));
    let context = if context.is_empty() { String::from("body") } else { context };
    let f = fragment(vm, &html, &context);
    replace_all(vm, n, Some(f));
    Ok(Value::Undefined)
}

fn outer_html(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_node(vm, this)?;
    Ok(js(&doc(vm).outer_html(n)))
}

fn set_outer_html(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let html = string(vm, &arg(args, 0))?;
    let Some(p) = doc(vm).parent(n) else { return Ok(Value::Undefined) };
    if p == 0 {
        return Err(dom_exception(vm, "NoModificationAllowedError", "This element's parent is of type '#document'."));
    }
    let context = String::from(doc(vm).name(p));
    let f = fragment(vm, &html, if context.is_empty() { "body" } else { &context });
    let next = doc(vm).next_sibling(n);
    remove(vm, n);
    pre_insert(vm, p, f, next)?;
    Ok(Value::Undefined)
}

fn set_outer_html_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let s = string(vm, &arg(args, 0))?;
    let Some(p) = doc(vm).parent(n) else { return Ok(Value::Undefined) };
    let t = host(vm).doc.create(Data::Text(s));
    let next = doc(vm).next_sibling(n);
    remove(vm, n);
    pre_insert(vm, p, t, next)?;
    Ok(Value::Undefined)
}

/// Where insertAdjacent* puts things: (parent, before).
fn adjacent(vm: &mut Vm, n: NodeId, place: &str) -> Result<(NodeId, Option<NodeId>), Value> {
    let d = doc(vm);
    match place.to_ascii_lowercase().as_str() {
        "beforebegin" => match d.parent(n) {
            Some(p) => Ok((p, Some(n))),
            None => Err(dom_exception(vm, "NoModificationAllowedError", "The element has no parent.")),
        },
        "afterbegin" => Ok((n, d.children(n).first().copied())),
        "beforeend" => Ok((n, None)),
        "afterend" => match d.parent(n) {
            Some(p) => Ok((p, d.next_sibling(n))),
            None => Err(dom_exception(vm, "NoModificationAllowedError", "The element has no parent.")),
        },
        _ => Err(dom_exception(vm, "SyntaxError", &format!("The value provided ('{place}') is not one of 'beforeBegin', 'afterBegin', 'beforeEnd', or 'afterEnd'."))),
    }
}

fn insert_adjacent_html(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let place = str_arg(vm, args, 0, "insertAdjacentHTML")?;
    let html = str_arg(vm, args, 1, "insertAdjacentHTML")?;
    let (p, before) = adjacent(vm, n, &place)?;
    let context = String::from(doc(vm).name(p));
    let f = fragment(vm, &html, if context.is_empty() { "body" } else { &context });
    pre_insert(vm, p, f, before)?;
    Ok(Value::Undefined)
}

fn insert_adjacent_element(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let place = str_arg(vm, args, 0, "insertAdjacentElement")?;
    let e = node_arg(vm, args, 1, "insertAdjacentElement")?;
    let (p, before) = adjacent(vm, n, &place)?;
    pre_insert(vm, p, e, before)?;
    Ok(arg(args, 1))
}

fn insert_adjacent_text(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let place = str_arg(vm, args, 0, "insertAdjacentText")?;
    let s = str_arg(vm, args, 1, "insertAdjacentText")?;
    let (p, before) = adjacent(vm, n, &place)?;
    let t = host(vm).doc.create(Data::Text(s));
    pre_insert(vm, p, t, before)?;
    Ok(Value::Undefined)
}

// --------------------------------------------------------------------------------- reflected attributes

fn reflected(vm: &mut Vm, this: &Value, f: Gc) -> Result<(NodeId, String), Value> {
    let n = this_element(vm, this)?;
    let name = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => String::new(),
    };
    Ok((n, name))
}

fn reflect_str(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    Ok(js(&attr(vm, n, &name).unwrap_or_default()))
}

fn reflect_set_str(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, &name, &v);
    Ok(Value::Undefined)
}

fn reflect_bool(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    Ok(Value::Bool(attr(vm, n, &name).is_some()))
}

fn reflect_set_bool(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    if arg(args, 0).truthy() {
        set_attr(vm, n, &name, "");
    } else {
        remove_attr(vm, n, &name);
    }
    Ok(Value::Undefined)
}

/// An address attribute's value resolved against the page's.
pub fn resolve(vm: &Vm, v: &str) -> String {
    match url::Url::parse(v, Some(&host_ref(vm).base)) {
        Some(u) => u.href(),
        None => String::from(v),
    }
}

fn reflect_url(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    Ok(match attr(vm, n, &name) {
        Some(v) => js(&resolve(vm, &v)),
        None => js(""),
    })
}

fn reflect_nullable(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    Ok(attr(vm, n, &name).map_or(Value::Null, |v| js(&v)))
}

fn reflect_set_nullable(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    let v = arg(args, 0);
    if matches!(v, Value::Null) {
        remove_attr(vm, n, &name);
    } else {
        let s = string(vm, &v)?;
        set_attr(vm, n, &name, &s);
    }
    Ok(Value::Undefined)
}

fn reflect_num(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    let d = match vm.data2(f) {
        Value::Num(d) => d,
        _ => 0.0,
    };
    Ok(Value::Num(attr(vm, n, &name).and_then(|v| parse_int(&v)).map_or(d, |v| v as f64)))
}

fn reflect_set_num(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let (n, name) = reflected(vm, this, f)?;
    let v = vm.to_int32(&arg(args, 0))?;
    set_attr(vm, n, &name, &format!("{v}"));
    Ok(Value::Undefined)
}

/// HTML's rules for parsing integers: spaces, a sign, digits (what follows them ignored).
fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    let v: i64 = digits.parse().ok()?;
    Some(if neg { -v } else { v })
}

// -------------------------------------------------------------------------------- HTMLElement's own

fn tab_index(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let focusable = matches!(doc(vm).name(n), "a" | "button" | "input" | "select" | "textarea" | "iframe" | "summary");
    Ok(Value::Num(attr(vm, n, "tabindex").and_then(|v| parse_int(&v)).map_or(if focusable { 0.0 } else { -1.0 }, |v| v as f64)))
}

fn set_tab_index(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = vm.to_int32(&arg(args, 0))?;
    set_attr(vm, n, "tabindex", &format!("{v}"));
    Ok(Value::Undefined)
}

fn draggable(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Bool(match attr(vm, n, "draggable").as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => matches!(doc(vm).name(n), "img") || (doc(vm).name(n) == "a" && attr(vm, n, "href").is_some()),
    }))
}

fn set_draggable(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    set_attr(vm, n, "draggable", if arg(args, 0).truthy() { "true" } else { "false" });
    Ok(Value::Undefined)
}

fn content_editable(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(match attr(vm, n, "contenteditable").as_deref() {
        Some("" | "true") => "true",
        Some("false") => "false",
        Some("plaintext-only") => "plaintext-only",
        _ => "inherit",
    }))
}

fn set_content_editable(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    if v == "inherit" {
        remove_attr(vm, n, "contenteditable");
    } else {
        set_attr(vm, n, "contenteditable", &v);
    }
    Ok(Value::Undefined)
}

fn is_content_editable(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let mut at = Some(n);
    while let Some(e) = at {
        match d.element(e).and_then(|el| el.attr("contenteditable")) {
            Some("" | "true" | "plaintext-only") => return Ok(Value::Bool(true)),
            Some("false") => return Ok(Value::Bool(false)),
            _ => {}
        }
        at = d.parent_element(e);
    }
    Ok(Value::Bool(false))
}

/// Where a node's box is (x, y, width, height), if it has one: the page laid out again first if it changed.
fn rect(vm: &mut Vm, n: NodeId) -> Option<(f32, f32, f32, f32)> {
    super::fresh_layout(vm);
    host_ref(vm).rects.get(n).copied().flatten()
}

fn offset_parent(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if rect(vm, n).is_none() {
        return Ok(Value::Null);
    }
    let body = doc(vm).find("body");
    Ok(wrap_opt(vm, body))
}

fn offset_top(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(rect(vm, n).map_or(0.0, |r| crate::num::round(r.1)) as f64))
}

fn offset_left(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(rect(vm, n).map_or(0.0, |r| crate::num::round(r.0)) as f64))
}

fn offset_width(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(rect(vm, n).map_or(0.0, |r| crate::num::round(r.2)) as f64))
}

fn offset_height(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(rect(vm, n).map_or(0.0, |r| crate::num::round(r.3)) as f64))
}

fn client_width(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if doc(vm).name(n) == "html" {
        return Ok(Value::Num(host_ref(vm).view.0 as f64));
    }
    offset_width(vm, this, &[], _f)
}

fn client_height(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if doc(vm).name(n) == "html" {
        return Ok(Value::Num(host_ref(vm).view.1 as f64));
    }
    offset_height(vm, this, &[], _f)
}

fn scroll_width(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    client_width(vm, this, &[], f)
}

fn scroll_height(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let n = this_element(vm, this)?;
    if matches!(doc(vm).name(n), "html" | "body") {
        super::fresh_layout(vm);
        let tall = host_ref(vm).rects.get(0).copied().flatten().map_or(0.0, |r| r.3);
        return Ok(Value::Num(tall.max(host_ref(vm).view.1) as f64));
    }
    client_height(vm, this, &[], f)
}

fn scroll_top(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if matches!(doc(vm).name(n), "html" | "body") {
        return Ok(Value::Num(host_ref(vm).scroll.1 as f64));
    }
    Ok(Value::Num(0.0))
}

fn set_scroll_top(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if matches!(doc(vm).name(n), "html" | "body") {
        let y = vm.to_number(&arg(args, 0))?;
        host(vm).scroll.1 = y.max(0.0) as f32;
    }
    Ok(Value::Undefined)
}

/// A DOMRect object.
pub fn dom_rect(vm: &mut Vm, x: f64, y: f64, w: f64, h: f64) -> Value {
    let p = host_ref(vm).proto("DOMRect").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let g = vm.alloc(Some(p), Class::Ordinary);
    for (k, v) in [("x", x), ("y", y), ("width", w), ("height", h), ("top", y.min(y + h)), ("left", x.min(x + w)), ("right", x.max(x + w)), ("bottom", y.max(y + h))] {
        vm.put(g, k, Value::Num(v), ALL);
    }
    Value::Obj(g)
}

fn dom_rect_new(vm: &mut Vm, _nt: &Value, args: &[Value], _f: Gc) -> R {
    let mut v = [0.0; 4];
    for (i, x) in v.iter_mut().enumerate() {
        let a = arg(args, i);
        *x = if a.is_undefined() { 0.0 } else { vm.to_number(&a)? };
    }
    Ok(dom_rect(vm, v[0], v[1], v[2], v[3]))
}

fn rect_to_json(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Ok(Value::Undefined) };
    let o = vm.new_object();
    for k in ["x", "y", "width", "height", "top", "right", "bottom", "left"] {
        let v = get(vm, *g, k)?;
        vm.put(o, k, v, ALL);
    }
    Ok(Value::Obj(o))
}

fn bounding_rect(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let (sx, sy) = host_ref(vm).scroll;
    let r = rect(vm, n).unwrap_or((0.0, 0.0, 0.0, 0.0));
    let (x, y) = if rect(vm, n).is_some() { (r.0 - sx, r.1 - sy) } else { (0.0, 0.0) };
    Ok(dom_rect(vm, x as f64, y as f64, r.2 as f64, r.3 as f64))
}

fn empty_rect(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(dom_rect(vm, 0.0, 0.0, 0.0, 0.0))
}

fn client_rects(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let n = this_element(vm, this)?;
    if rect(vm, n).is_none() {
        return Ok(Value::Obj(vm.new_array(Vec::new())));
    }
    let r = bounding_rect(vm, this, args, f)?;
    Ok(Value::Obj(vm.new_array(vec![r])))
}

fn scroll_into_view(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if let Some(r) = rect(vm, n) {
        host(vm).scroll.1 = r.1.max(0.0);
    }
    Ok(Value::Undefined)
}

fn click(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if attr(vm, n, "disabled").is_some() && matches!(doc(vm).name(n), "button" | "input" | "select" | "textarea") {
        return Ok(Value::Undefined);
    }
    let init = EventInit::new("click", "PointerEvent", true, true).with("detail", super::events::EvValue::Num(1.0));
    let ok = super::events::fire(vm, this, &init);
    if ok {
        activate(vm, n);
    }
    Ok(Value::Undefined)
}

/// What a click on an element does when nothing cancelled it: a checkbox ticked, a form sent, a link followed.
pub fn activate(vm: &mut Vm, n: NodeId) {
    let name = String::from(doc(vm).name(n));
    let ty = attr(vm, n, "type").unwrap_or_default().to_ascii_lowercase();
    match name.as_str() {
        "input" if ty == "checkbox" => {
            if attr(vm, n, "checked").is_some() {
                remove_attr(vm, n, "checked");
            } else {
                set_attr(vm, n, "checked", "");
            }
            changed_events(vm, n);
        }
        "input" if ty == "radio" => {
            if attr(vm, n, "checked").is_none() {
                uncheck_group(vm, n);
                set_attr(vm, n, "checked", "");
                changed_events(vm, n);
            }
        }
        "input" | "button" if matches!(ty.as_str(), "submit" | "image") || (name == "button" && ty.is_empty()) => {
            if let Some(f) = form_of(vm, n) {
                submit_form(vm, f, Some(n));
            }
        }
        "a" | "area" => {
            if let Some(h) = attr(vm, n, "href") {
                let to = resolve(vm, &h);
                if let Some(u) = url::Url::parse(&to, None) {
                    super::window::navigate(vm, u);
                }
            }
        }
        _ => {}
    }
}

fn changed_events(vm: &mut Vm, n: NodeId) {
    let w = wrap(vm, n);
    super::events::fire(vm, &w, &EventInit::new("input", "Event", true, false));
    super::events::fire(vm, &w, &EventInit::new("change", "Event", true, false));
}

/// The other radio buttons of a button's group, unchecked.
fn uncheck_group(vm: &mut Vm, n: NodeId) {
    let Some(name) = attr(vm, n, "name") else { return };
    let form = form_of(vm, n);
    let d = doc(vm);
    let others: Vec<NodeId> = d
        .descendants(0)
        .filter(|&o| o != n && d.name(o) == "input" && d.element(o).and_then(|e| e.attr("type")).is_some_and(|t| t.eq_ignore_ascii_case("radio")) && d.element(o).and_then(|e| e.attr("name")) == Some(name.as_str()))
        .collect();
    for o in others {
        if form_of(vm, o) == form {
            remove_attr(vm, o, "checked");
        }
    }
}

/// The form a control belongs to (its form attribute's, or the one around it).
pub fn form_of(vm: &Vm, n: NodeId) -> Option<NodeId> {
    let d = doc(vm);
    if let Some(id) = d.element(n).and_then(|e| e.attr("form")) {
        return d.descendants(0).find(|&f| d.name(f) == "form" && d.element(f).and_then(|e| e.attr("id")) == Some(id));
    }
    d.ancestor_named(n, "form")
}

/// A form sent (its submit event first, unless a script calls submit() itself).
pub fn submit_form(vm: &mut Vm, f: NodeId, submitter: Option<NodeId>) {
    let w = wrap(vm, f);
    let s = wrap_opt(vm, submitter);
    let init = EventInit::new("submit", "SubmitEvent", true, true).with("submitter", super::events::EvValue::Node(node_of(vm, &s)));
    if super::events::fire(vm, &w, &init) {
        host(vm).rescan = true;
        super::window::request_submit(vm, f, submitter);
    }
}

fn focus(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let old = host_ref(vm).focus;
    if old == Some(n) {
        return Ok(Value::Undefined);
    }
    if let Some(o) = old {
        let ow = wrap(vm, o);
        host(vm).focus = None;
        super::events::fire(vm, &ow, &EventInit::new("blur", "FocusEvent", false, false));
        super::events::fire(vm, &ow, &EventInit::new("focusout", "FocusEvent", true, false));
    }
    host(vm).focus = Some(n);
    super::events::fire(vm, this, &EventInit::new("focus", "FocusEvent", false, false));
    super::events::fire(vm, this, &EventInit::new("focusin", "FocusEvent", true, false));
    Ok(Value::Undefined)
}

fn blur(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if host_ref(vm).focus == Some(n) {
        host(vm).focus = None;
        super::events::fire(vm, this, &EventInit::new("blur", "FocusEvent", false, false));
        super::events::fire(vm, this, &EventInit::new("focusout", "FocusEvent", true, false));
    }
    Ok(Value::Undefined)
}

fn animate(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    // An animation that's already over (this browser doesn't animate): its promises resolved.
    let a = vm.new_object();
    for name in ["cancel", "finish", "play", "pause", "reverse", "commitStyles", "persist", "updatePlaybackRate"] {
        method(vm, a, name, 0, ignore_fn);
    }
    let p = vm.new_promise();
    vm.fulfill_promise(p, Value::Obj(a));
    vm.put(a, "finished", Value::Obj(p), ALL);
    let p2 = vm.new_promise();
    vm.fulfill_promise(p2, Value::Obj(a));
    vm.put(a, "ready", Value::Obj(p2), ALL);
    vm.put(a, "playState", js("finished"), ALL);
    vm.put(a, "onfinish", Value::Null, ALL);
    vm.put(a, "currentTime", Value::Num(0.0), ALL);
    Ok(Value::Obj(a))
}

// ----------------------------------------------------------------------------------------- shadow roots

const SHADOW: u32 = 0x2000_0000;

fn attach_shadow(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if doc(vm).shadows.iter().any(|(h, _)| *h == n) {
        return Err(dom_exception(vm, "NotSupportedError", "Shadow root cannot be created on a host which already hosts a shadow tree."));
    }
    let mode = match arg(args, 0) {
        Value::Obj(o) => get(vm, o, "mode")?,
        _ => Value::Undefined,
    };
    let mode = string(vm, &mode)?;
    // The shadow tree: a fragment the host's box shows (in place of the host's own children, which its <slot>s
    // show where they are).
    let f = host(vm).doc.create(Data::Fragment);
    host(vm).doc.shadows.push((n, f));
    let g = wrap(vm, f);
    if let Value::Obj(sg) = g {
        let w = wrap(vm, n);
        set_slot(vm, sg, host_ref(vm).syms.target, w);
        set_slot(vm, sg, host_ref(vm).syms.data, js(&mode));
        host(vm).cache.insert((n, SHADOW), sg);
    }
    host(vm).changed();
    Ok(g)
}

fn shadow_root(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    match host_ref(vm).cache.get(&(n, SHADOW)).copied() {
        Some(g) if matches!(slot(vm, &Value::Obj(g), host_ref(vm).syms.data), Value::Str(ref s) if s.to_rust() == "open") => Ok(Value::Obj(g)),
        _ => Ok(Value::Null),
    }
}

fn shadow_host(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(slot(vm, this, host_ref(vm).syms.target))
}

fn shadow_mode(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(slot(vm, this, host_ref(vm).syms.data))
}

fn attach_internals(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let p = host_ref(vm).proto("ElementInternals").unwrap_or_else(|| vm.proto(I::ObjectProto));
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

fn element_states(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Ok(Value::Undefined) };
    let existing = get(vm, *g, "__states")?;
    if existing.is_object() {
        return Ok(existing);
    }
    let set_ctor = get(vm, host_ref(vm).window, "Set")?;
    let s = vm.construct(&set_ctor, &[], &set_ctor)?;
    vm.put(*g, "__states", s.clone(), 0);
    Ok(s)
}

// -------------------------------------------------------------------------------------- custom elements

fn html_element_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    // A custom element's constructor's super(): the element being upgraded, or a new one of its name.
    let def = host_ref(vm).custom.iter().find(|(_, c)| js::value::strict_equals(c, nt)).map(|(n, _)| n.clone());
    let Some(name) = def else { return Err(vm.type_error("Illegal constructor")) };
    let p = proto_from(vm, nt, "HTMLElement")?;
    let upgrading = UPGRADING.load(core::sync::atomic::Ordering::Relaxed);
    let n = if upgrading != usize::MAX {
        UPGRADING.store(usize::MAX, core::sync::atomic::Ordering::Relaxed);
        upgrading
    } else {
        host(vm).doc.create(Data::Element(Element { name, attrs: Vec::new() }))
    };
    let w = wrap(vm, n);
    if let Value::Obj(g) = w {
        vm.heap.get_mut(g).proto = Some(p);
    }
    Ok(w)
}

/// The element a custom element's constructor is upgrading (usize::MAX: none).
static UPGRADING: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(usize::MAX);

/// Marks elements already upgraded (by their objects' hidden flag).
fn upgraded(vm: &Vm, n: NodeId) -> bool {
    match host_ref(vm).wrappers.get(n) {
        Some(Some(g)) => matches!(slot(vm, &Value::Obj(*g), host_ref(vm).syms.event), Value::Bool(true)),
        _ => false,
    }
}

/// Runs a custom element's constructor on an element of its name (once).
pub fn upgrade(vm: &mut Vm, n: NodeId) {
    let name = String::from(doc(vm).name(n));
    let Some(c) = host_ref(vm).custom.iter().find(|(x, _)| *x == name).map(|(_, c)| c.clone()) else { return };
    if upgraded(vm, n) {
        return;
    }
    let w = wrap(vm, n);
    if let Value::Obj(g) = w {
        set_slot(vm, g, host_ref(vm).syms.event, Value::Bool(true));
        if let Value::Obj(cg) = &c
            && let Ok(Value::Obj(p)) = get(vm, *cg, "prototype")
        {
            vm.heap.get_mut(g).proto = Some(p);
        }
    }
    UPGRADING.store(n, core::sync::atomic::Ordering::Relaxed);
    let r = vm.construct(&c, &[], &c);
    UPGRADING.store(usize::MAX, core::sync::atomic::Ordering::Relaxed);
    if let Err(e) = r {
        super::report(vm, &e, "a custom element's constructor");
        return;
    }
    // Its observed attributes, as they are.
    let attrs: Vec<(String, String)> = element_of(vm, n).map(|e| e.attrs.clone()).unwrap_or_default();
    for (k, v) in attrs {
        if observed(vm, &name, &k) {
            callback(vm, n, "attributeChangedCallback", &[js(&k), Value::Null, js(&v), Value::Null]);
        }
    }
}

/// Whether a custom element's class observes an attribute (its static observedAttributes).
fn observed(vm: &mut Vm, name: &str, attr: &str) -> bool {
    let Some(Value::Obj(c)) = host_ref(vm).custom.iter().find(|(x, _)| x == name).map(|(_, c)| c.clone()) else { return false };
    let Ok(list) = get(vm, c, "observedAttributes") else { return false };
    if list.is_nullish() {
        return false;
    }
    let Ok(items) = vm.iterate_to_vec(&list) else { return false };
    items.iter().any(|v| matches!(v, Value::Str(s) if s.to_rust() == attr))
}

/// Calls one of a custom element's lifecycle methods (connectedCallback...), if it has it.
fn callback(vm: &mut Vm, n: NodeId, which: &str, args: &[Value]) {
    if !upgraded(vm, n) {
        return;
    }
    let w = wrap(vm, n);
    let Value::Obj(g) = w else { return };
    let Ok(f) = get(vm, g, which) else { return };
    if vm.is_callable(&f)
        && let Err(e) = vm.call(&f, w, args)
    {
        super::report(vm, &e, which);
    }
}

fn ce_define(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = str_arg(vm, args, 0, "define")?;
    let c = arg(args, 1);
    if !vm.is_constructor(&c) {
        return Err(vm.type_error("Failed to execute 'define' on 'CustomElementRegistry': The provided value cannot be converted to a constructor."));
    }
    if !name.contains('-') || name.starts_with(|ch: char| !ch.is_ascii_lowercase()) || name.chars().any(|ch| ch.is_ascii_uppercase()) {
        return Err(dom_exception(vm, "SyntaxError", &format!("Failed to execute 'define' on 'CustomElementRegistry': \"{name}\" is not a valid custom element name")));
    }
    if host_ref(vm).custom.iter().any(|(n, _)| *n == name) {
        return Err(dom_exception(vm, "NotSupportedError", &format!("Failed to execute 'define' on 'CustomElementRegistry': the name \"{name}\" has already been used with this registry")));
    }
    host(vm).custom.push((name.clone(), c));
    // Elements of its name already in the page: upgraded, and told they're connected.
    let d = doc(vm);
    let found: Vec<NodeId> = d.descendants(0).filter(|&e| d.name(e) == name).collect();
    for e in found {
        upgrade(vm, e);
        callback(vm, e, "connectedCallback", &[]);
    }
    // Promises from whenDefined.
    let key = (usize::MAX, 0x3000_0000 + (super::events::fnv(&name) & 0x0fff_ffff));
    if let Some(p) = host(vm).cache.remove(&key) {
        let c = host_ref(vm).custom.iter().find(|(n, _)| *n == name).map(|(_, c)| c.clone()).unwrap_or(Value::Undefined);
        vm.fulfill_promise(p, c);
    }
    Ok(Value::Undefined)
}

fn ce_get(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = string(vm, &arg(args, 0))?;
    Ok(host_ref(vm).custom.iter().find(|(n, _)| *n == name).map_or(Value::Undefined, |(_, c)| c.clone()))
}

fn ce_get_name(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let c = arg(args, 0);
    Ok(host_ref(vm).custom.iter().find(|(_, x)| js::value::strict_equals(x, &c)).map_or(Value::Null, |(n, _)| js(n)))
}

fn ce_when_defined(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = string(vm, &arg(args, 0))?;
    if let Some(c) = host_ref(vm).custom.iter().find(|(n, _)| *n == name).map(|(_, c)| c.clone()) {
        let p = vm.new_promise();
        vm.fulfill_promise(p, c);
        return Ok(Value::Obj(p));
    }
    let key = (usize::MAX, 0x3000_0000 + (super::events::fnv(&name) & 0x0fff_ffff));
    if let Some(p) = host_ref(vm).cache.get(&key) {
        return Ok(Value::Obj(*p));
    }
    let p = vm.new_promise();
    host(vm).cache.insert(key, p);
    Ok(Value::Obj(p))
}

fn ce_upgrade(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    if let Some(n) = node_of(vm, &arg(args, 0)) {
        let all: Vec<NodeId> = doc(vm).descendants(n).collect();
        for e in all {
            upgrade(vm, e);
        }
    }
    Ok(Value::Undefined)
}

// ----------------------------------------------------------------------------------------- Document

fn document_members(vm: &mut Vm, dp: Gc) {
    parent_node_methods(vm, dp);
    for name in super::events::HANDLERS {
        handler_property(vm, dp, name);
    }
    attribute(vm, dp, "documentElement", document_element, None);
    attribute(vm, dp, "head", document_head, None);
    attribute(vm, dp, "body", document_body, Some(set_document_body));
    attribute(vm, dp, "title", document_title, Some(set_document_title));
    attribute(vm, dp, "URL", document_url, None);
    attribute(vm, dp, "documentURI", document_url, None);
    attribute(vm, dp, "domain", document_domain, Some(ignore_fn));
    attribute(vm, dp, "referrer", empty_string, None);
    attribute(vm, dp, "cookie", document_cookie, Some(set_document_cookie));
    attribute(vm, dp, "readyState", ready_state, None);
    attribute(vm, dp, "characterSet", utf8, None);
    attribute(vm, dp, "charset", utf8, None);
    attribute(vm, dp, "inputEncoding", utf8, None);
    attribute(vm, dp, "contentType", content_type, None);
    attribute(vm, dp, "compatMode", compat_mode, None);
    attribute(vm, dp, "doctype", doctype, None);
    attribute(vm, dp, "defaultView", default_view, None);
    attribute(vm, dp, "activeElement", active_element, None);
    attribute(vm, dp, "visibilityState", visible, None);
    attribute(vm, dp, "webkitVisibilityState", visible, None);
    attribute(vm, dp, "hidden", false_fn, None);
    attribute(vm, dp, "webkitHidden", false_fn, None);
    attribute(vm, dp, "fullscreenElement", null_fn, None);
    attribute(vm, dp, "fullscreenEnabled", false_fn, None);
    attribute(vm, dp, "pointerLockElement", null_fn, None);
    attribute(vm, dp, "pictureInPictureElement", null_fn, None);
    attribute(vm, dp, "scrollingElement", document_element, None);
    attribute(vm, dp, "currentScript", current_script, None);
    attribute(vm, dp, "lastModified", last_modified, None);
    attribute(vm, dp, "designMode", design_mode, Some(ignore_fn));
    attribute(vm, dp, "dir", document_dir, Some(ignore_fn));
    attribute(vm, dp, "implementation", implementation, None);
    for (name, sel) in [("forms", "form"), ("images", "img"), ("links", "a[href], area[href]"), ("scripts", "script"), ("embeds", "embed"), ("plugins", "embed"), ("anchors", "a[name]")] {
        attribute_with(vm, dp, name, document_collection, None, js(sel));
    }
    attribute(vm, dp, "styleSheets", style_sheets, None);
    attribute(vm, dp, "fonts", fonts, None);
    attribute(vm, dp, "location", document_location, Some(set_document_location));
    method(vm, dp, "createElement", 1, create_element);
    method(vm, dp, "createElementNS", 2, create_element_ns);
    method(vm, dp, "createTextNode", 1, create_text_node);
    method(vm, dp, "createComment", 1, create_comment);
    method(vm, dp, "createDocumentFragment", 0, create_document_fragment);
    method(vm, dp, "createEvent", 1, create_event);
    method(vm, dp, "createRange", 0, range_new_fn);
    method(vm, dp, "createTreeWalker", 1, create_tree_walker);
    method(vm, dp, "createNodeIterator", 1, create_tree_walker);
    method(vm, dp, "createAttribute", 1, create_attribute);
    method(vm, dp, "getElementById", 1, get_element_by_id);
    method(vm, dp, "getElementsByName", 1, get_elements_by_name);
    method(vm, dp, "getElementsByTagName", 1, get_elements_by_tag_name);
    method(vm, dp, "getElementsByTagNameNS", 2, get_elements_by_tag_name_ns);
    method(vm, dp, "getElementsByClassName", 1, get_elements_by_class_name);
    method(vm, dp, "elementFromPoint", 2, element_from_point);
    method(vm, dp, "elementsFromPoint", 2, elements_from_point);
    method(vm, dp, "hasFocus", 0, true_fn);
    method(vm, dp, "write", 0, document_write);
    method(vm, dp, "writeln", 0, document_writeln);
    method(vm, dp, "open", 0, document_open);
    method(vm, dp, "close", 0, ignore_fn);
    method(vm, dp, "importNode", 1, import_node);
    method(vm, dp, "adoptNode", 1, adopt_node);
    method(vm, dp, "getSelection", 0, get_selection);
    method(vm, dp, "execCommand", 0, false_fn);
    method(vm, dp, "queryCommandSupported", 0, false_fn);
    method(vm, dp, "exitFullscreen", 0, rejected_fn);
    method(vm, dp, "exitPointerLock", 0, ignore_fn);
    method(vm, dp, "startViewTransition", 1, start_view_transition);
}

fn document_element(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let h = element_children(doc(vm), 0).first().copied();
    Ok(wrap_opt(vm, h))
}

fn document_head(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let h = doc(vm).find("head");
    Ok(wrap_opt(vm, h))
}

fn document_body(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let b = doc(vm).find("body").or_else(|| doc(vm).find("frameset"));
    Ok(wrap_opt(vm, b))
}

fn set_document_body(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let n = node_arg(vm, args, 0, "body")?;
    let html = element_children(doc(vm), 0).first().copied();
    if let Some(h) = html {
        match doc(vm).find("body") {
            Some(old) => {
                let next = doc(vm).next_sibling(old);
                remove(vm, old);
                pre_insert(vm, h, n, next)?;
            }
            None => pre_insert(vm, h, n, None)?,
        }
    }
    Ok(Value::Undefined)
}

fn document_title(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let d = doc(vm);
    let t = d.find("title").map(|t| d.text_of(t)).unwrap_or_default();
    Ok(js(&t.split_whitespace().collect::<Vec<&str>>().join(" ")))
}

fn set_document_title(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    let t = match doc(vm).find("title") {
        Some(t) => t,
        None => {
            let Some(head) = doc(vm).find("head") else { return Ok(Value::Undefined) };
            let t = host(vm).doc.create(Data::Element(Element { name: String::from("title"), attrs: Vec::new() }));
            pre_insert(vm, head, t, None)?;
            t
        }
    };
    let text = host(vm).doc.create(Data::Text(s));
    replace_all(vm, t, Some(text));
    host(vm).title_changed = true;
    Ok(Value::Undefined)
}

fn document_url(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js(&host_ref(vm).url.href()))
}

fn document_domain(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js(host_ref(vm).url.host.as_deref().unwrap_or("")))
}

fn document_cookie(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let s: Vec<String> = host_ref(vm).cookies.iter().map(|(k, v)| if k.is_empty() { v.clone() } else { format!("{k}={v}") }).collect();
    Ok(js(&s.join("; ")))
}

fn set_document_cookie(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    let mut parts = s.split(';');
    let first = parts.next().unwrap_or("").trim();
    let (k, v) = first.split_once('=').map_or(("", first), |(k, v)| (k.trim(), v.trim()));
    // (An expiry in the past, or max-age 0 or less, deletes it.)
    let gone = parts.any(|p| {
        let p = p.trim().to_ascii_lowercase();
        p.strip_prefix("max-age=").is_some_and(|a| a.trim().parse::<i64>().is_ok_and(|a| a <= 0)) || (p.starts_with("expires=") && (p.contains("1970") || p.contains("thu, 01 jan")))
    });
    let h = host(vm);
    h.cookies.retain(|(name, _)| name != k);
    if !gone {
        h.cookies.push((String::from(k), String::from(v)));
    }
    Ok(Value::Undefined)
}

fn ready_state(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let h = host_ref(vm);
    Ok(js(if h.loaded { "complete" } else if h.current_script.is_some() && !h.started.is_empty() { "loading" } else { "interactive" }))
}

fn utf8(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("UTF-8"))
}

fn content_type(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("text/html"))
}

fn compat_mode(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js(if doc(vm).quirks { "BackCompat" } else { "CSS1Compat" }))
}

fn doctype(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    if doc(vm).quirks {
        return Ok(Value::Null);
    }
    let p = host_ref(vm).proto("DocumentType").unwrap_or_else(|| vm.proto(I::ObjectProto));
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

fn default_view(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(Value::Obj(host_ref(vm).window))
}

fn active_element(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let f = host_ref(vm).focus.or_else(|| doc(vm).find("body"));
    Ok(wrap_opt(vm, f))
}

fn visible(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("visible"))
}

fn current_script(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let s = host_ref(vm).current_script;
    Ok(wrap_opt(vm, s))
}

fn last_modified(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("01/01/2026 00:00:00"))
}

fn design_mode(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("off"))
}

fn document_dir(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let html = element_children(doc(vm), 0).first().copied();
    Ok(js(&html.and_then(|h| attr(vm, h, "dir")).unwrap_or_default()))
}

fn implementation(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    method(vm, o, "hasFeature", 0, true_fn);
    method(vm, o, "createHTMLDocument", 1, create_html_document);
    method(vm, o, "createDocument", 3, create_html_document);
    method(vm, o, "createDocumentType", 3, null_fn);
    Ok(Value::Obj(o))
}

/// A new document (implementation.createHTMLDocument, DOMParser): a fragment holding <html>, <head> and <body>,
/// whose own methods (body, head, querySelector...) work as a document's.
fn new_document(vm: &mut Vm, html: &str) -> Value {
    let parsed = crate::html::parse_with(html, false);
    let f = host(vm).doc.create(Data::Fragment);
    let kids: Vec<NodeId> = parsed.children(0).to_vec();
    for k in kids {
        let c = import(&mut host(vm).doc, &parsed, k);
        host(vm).doc.insert(f, c, None);
    }
    let w = wrap(vm, f);
    // (Its document-ish members: an object whose prototype is the fragment, with body, head, documentElement.)
    let Value::Obj(fg) = w else { return w };
    let d = vm.alloc(Some(fg), Class::Ordinary);
    let html_el = element_children(doc(vm), f).first().copied();
    let head = html_el.and_then(|h| element_children(doc(vm), h).into_iter().find(|&c| doc(vm).name(c) == "head"));
    let body = html_el.and_then(|h| element_children(doc(vm), h).into_iter().find(|&c| doc(vm).name(c) == "body"));
    let (hw, headw, bodyw) = (wrap_opt(vm, html_el), wrap_opt(vm, head), wrap_opt(vm, body));
    vm.put(d, "documentElement", hw, ALL);
    vm.put(d, "head", headw, ALL);
    vm.put(d, "body", bodyw, ALL);
    let title = head.and_then(|h| doc(vm).descendants(h).find(|&t| doc(vm).name(t) == "title")).map(|t| doc(vm).text_of(t)).unwrap_or_default();
    vm.put(d, "title", js(&title), ALL);
    let dp = host_ref(vm).proto("Document");
    if let Some(dp) = dp {
        for name in ["createElement", "createTextNode", "createComment", "createDocumentFragment", "getElementById", "getElementsByTagName", "getElementsByClassName", "createElementNS", "importNode", "adoptNode"] {
            if let Ok(m) = get(vm, dp, name) {
                vm.put(d, name, m, HIDDEN);
            }
        }
    }
    Value::Obj(d)
}

fn create_html_document(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let title = match arg(args, 0) {
        Value::Undefined => String::new(),
        v => format!("<title>{}</title>", string(vm, &v)?),
    };
    Ok(new_document(vm, &format!("<!doctype html><html><head>{title}</head><body></body></html>")))
}

fn dom_parser_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "DOMParser")?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

fn parse_from_string(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = str_arg(vm, args, 0, "parseFromString")?;
    Ok(new_document(vm, &s))
}

fn xml_serializer_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "XMLSerializer")?;
    Ok(Value::Obj(vm.alloc(Some(p), Class::Ordinary)))
}

fn serialize_to_string(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let n = node_arg(vm, args, 0, "serializeToString")?;
    Ok(js(&doc(vm).outer_html(n)))
}

fn document_collection(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let sel = match vm.data(f) {
        Value::Str(s) => s.to_rust(),
        _ => return Ok(Value::Null),
    };
    let kind = match sel.as_str() {
        "form" | "img" | "script" | "embed" => ListKind::Tag(sel),
        _ => {
            let found = select(vm, 0, &sel, false)?;
            return Ok(node_list(vm, found));
        }
    };
    Ok(super::lists::live(vm, 0, kind))
}

fn style_sheets(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    // One sheet object per <style> and stylesheet <link> (with no rules a script can read yet).
    let d = doc(vm);
    let sheets: Vec<NodeId> = d.descendants(0).filter(|&n| d.name(n) == "style" || (d.name(n) == "link" && d.element(n).and_then(|e| e.attr("rel")).is_some_and(|r| r.to_ascii_lowercase().contains("stylesheet")))).collect();
    let mut items = Vec::new();
    for n in sheets {
        let o = vm.new_object();
        let w = wrap(vm, n);
        vm.put(o, "ownerNode", w, ALL);
        let rules = vm.new_array(Vec::new());
        vm.put(o, "cssRules", Value::Obj(rules), ALL);
        vm.put(o, "rules", Value::Obj(rules), ALL);
        vm.put(o, "disabled", Value::Bool(false), ALL);
        let href = attr(vm, n, "href").map_or(Value::Null, |h| js(&resolve(vm, &h)));
        vm.put(o, "href", href, ALL);
        method(vm, o, "insertRule", 2, zero_fn);
        method(vm, o, "deleteRule", 1, ignore_fn);
        method(vm, o, "addRule", 2, zero_fn);
        method(vm, o, "replaceSync", 1, ignore_fn);
        items.push(Value::Obj(o));
    }
    Ok(Value::Obj(vm.new_array(items)))
}

fn fonts(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    let p = vm.new_promise();
    vm.fulfill_promise(p, Value::Obj(o));
    vm.put(o, "ready", Value::Obj(p), ALL);
    vm.put(o, "status", js("loaded"), ALL);
    method(vm, o, "check", 1, true_fn);
    method(vm, o, "load", 1, fonts_load);
    method(vm, o, "add", 1, ignore_fn);
    method(vm, o, "addEventListener", 2, ignore_fn);
    method(vm, o, "removeEventListener", 2, ignore_fn);
    method(vm, o, "forEach", 1, ignore_fn);
    Ok(Value::Obj(o))
}

fn fonts_load(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    let a = vm.new_array(Vec::new());
    vm.fulfill_promise(p, Value::Obj(a));
    Ok(Value::Obj(p))
}

fn document_location(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let w = host_ref(vm).window;
    get(vm, w, "location")
}

fn set_document_location(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    let to = resolve(vm, &s);
    if let Some(u) = url::Url::parse(&to, None) {
        super::window::navigate(vm, u);
    }
    Ok(Value::Undefined)
}

/// A new element of a name (a custom element's constructor run for one that's defined).
pub fn create(vm: &mut Vm, name: &str) -> Result<Value, Value> {
    if let Some(c) = host_ref(vm).custom.iter().find(|(n, _)| n == name).map(|(_, c)| c.clone()) {
        return vm.construct(&c, &[], &c);
    }
    let n = host(vm).doc.create(Data::Element(Element { name: String::from(name), attrs: Vec::new() }));
    Ok(wrap(vm, n))
}

fn create_element(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = str_arg(vm, args, 0, "createElement")?;
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == ':') || name.contains(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '/' | '=')) {
        return Err(dom_exception(vm, "InvalidCharacterError", &format!("The tag name provided ('{name}') is not a valid name.")));
    }
    create(vm, &name.to_ascii_lowercase())
}

fn create_element_ns(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = str_arg(vm, args, 1, "createElementNS")?;
    let local = String::from(name.rsplit(':').next().unwrap_or(&name));
    let ns = string(vm, &arg(args, 0)).unwrap_or_default();
    let name = if ns.contains("xhtml") { local.to_ascii_lowercase() } else { local.clone() };
    let w = create(vm, &name)?;
    // (An SVG element made alone: its interface SVG's.)
    if ns.contains("svg")
        && let Value::Obj(g) = &w
    {
        let iface = SVG.iter().find(|(n, _)| *n == local).map_or("SVGElement", |(_, i)| i);
        if let Some(p) = host_ref(vm).proto(iface) {
            vm.heap.get_mut(*g).proto = Some(p);
        }
    }
    Ok(w)
}

fn create_text_node(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = str_arg(vm, args, 0, "createTextNode")?;
    let n = host(vm).doc.create(Data::Text(s));
    Ok(wrap(vm, n))
}

fn create_comment(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let s = string(vm, &arg(args, 0))?;
    let n = host(vm).doc.create(Data::Comment(s));
    Ok(wrap(vm, n))
}

fn create_document_fragment(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = host(vm).doc.create(Data::Fragment);
    Ok(wrap(vm, n))
}

fn create_attribute(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = str_arg(vm, args, 0, "createAttribute")?;
    let o = vm.new_object();
    vm.put(o, "name", js(&name.to_ascii_lowercase()), ALL);
    vm.put(o, "value", js(""), ALL);
    Ok(Value::Obj(o))
}

fn create_event(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let iface = str_arg(vm, args, 0, "createEvent")?;
    let name = match iface.to_ascii_lowercase().as_str() {
        "event" | "events" | "htmlevents" | "svgevents" => "Event",
        "customevent" => "CustomEvent",
        "mouseevent" | "mouseevents" => "MouseEvent",
        "keyboardevent" | "keyevents" => "KeyboardEvent",
        "uievent" | "uievents" => "UIEvent",
        "focusevent" => "FocusEvent",
        "messageevent" => "MessageEvent",
        "touchevent" => "TouchEvent",
        _ => return Err(dom_exception(vm, "NotSupportedError", &format!("The provided event type ('{iface}') is invalid."))),
    };
    let w = host_ref(vm).window;
    let c = get(vm, w, name)?;
    vm.construct(&c, &[js("")], &c)
}

fn get_elements_by_name(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let name = str_arg(vm, args, 0, "getElementsByName")?;
    Ok(super::lists::live(vm, 0, ListKind::Name(name)))
}

/// The deepest elements whose boxes hold a point (in the view's coordinates), innermost first.
fn at_point(vm: &mut Vm, args: &[Value]) -> Result<Vec<NodeId>, Value> {
    let x = vm.to_number(&arg(args, 0))? as f32;
    let y = vm.to_number(&arg(args, 1))? as f32;
    super::fresh_layout(vm);
    let (sx, sy) = host_ref(vm).scroll;
    let (px, py) = (x + sx, y + sy);
    let h = host_ref(vm);
    let mut hits: Vec<(usize, NodeId)> = Vec::new();
    for id in h.doc.descendants(0) {
        if h.doc.element(id).is_none() {
            continue;
        }
        if let Some(Some((rx, ry, rw, rh))) = h.rects.get(id)
            && px >= *rx
            && py >= *ry
            && px < rx + rw
            && py < ry + rh
        {
            let mut depth = 0;
            let mut at = h.doc.parent(id);
            while let Some(p) = at {
                depth += 1;
                at = h.doc.parent(p);
            }
            hits.push((depth, id));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(hits.into_iter().map(|(_, id)| id).collect())
}

fn element_from_point(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let hits = at_point(vm, args)?;
    Ok(wrap_opt(vm, hits.first().copied()))
}

fn elements_from_point(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let hits = at_point(vm, args)?;
    let items: Vec<Value> = hits.into_iter().map(|n| wrap(vm, n)).collect();
    Ok(Value::Obj(vm.new_array(items)))
}

/// document.write: what a script writes goes in after the script running (or the body's end, after the page has
/// loaded), read as HTML.
fn document_write(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let mut html = String::new();
    for a in args {
        html.push_str(&string(vm, a)?);
    }
    write_html(vm, &html)?;
    Ok(Value::Undefined)
}

fn document_writeln(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    document_write(vm, this, args, f)?;
    write_html(vm, "\n")?;
    Ok(Value::Undefined)
}

fn write_html(vm: &mut Vm, html: &str) -> Result<(), Value> {
    let (parent, before) = match host_ref(vm).current_script {
        Some(s) if doc(vm).parent(s).is_some() => (doc(vm).parent(s).unwrap_or(0), doc(vm).next_sibling(s)),
        _ => match doc(vm).find("body") {
            Some(b) => (b, None),
            None => return Ok(()),
        },
    };
    let context = String::from(doc(vm).name(parent));
    let f = fragment(vm, html, if context.is_empty() { "body" } else { &context });
    pre_insert(vm, parent, f, before)
}

fn document_open(_vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(this.clone())
}

fn import_node(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let n = node_arg(vm, args, 0, "importNode")?;
    let deep = arg(args, 1).truthy();
    let c = host(vm).doc.clone_node(n, deep);
    Ok(wrap(vm, c))
}

fn adopt_node(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let n = node_arg(vm, args, 0, "adoptNode")?;
    remove(vm, n);
    Ok(arg(args, 0))
}

fn get_selection(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    vm.put(o, "rangeCount", Value::Num(0.0), ALL);
    vm.put(o, "isCollapsed", Value::Bool(true), ALL);
    vm.put(o, "type", js("None"), ALL);
    vm.put(o, "anchorNode", Value::Null, ALL);
    vm.put(o, "focusNode", Value::Null, ALL);
    for name in ["removeAllRanges", "addRange", "collapse", "extend", "selectAllChildren", "empty", "collapseToEnd", "collapseToStart", "deleteFromDocument", "removeRange", "setBaseAndExtent"] {
        method(vm, o, name, 0, ignore_fn);
    }
    method(vm, o, "toString", 0, empty_string);
    method(vm, o, "getRangeAt", 1, range_new_fn);
    method(vm, o, "containsNode", 1, false_fn);
    Ok(Value::Obj(o))
}

fn start_view_transition(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    // The change made at once (no transition to show): its callback called, its promises resolved.
    let cb = arg(args, 0);
    let mut r = Value::Undefined;
    if vm.is_callable(&cb) {
        r = vm.call(&cb, Value::Undefined, &[])?;
    }
    let o = vm.new_object();
    let done = vm.promise_resolve(r)?;
    for name in ["finished", "ready", "updateCallbackDone"] {
        vm.put(o, name, Value::Obj(done), ALL);
    }
    method(vm, o, "skipTransition", 0, ignore_fn);
    Ok(Value::Obj(o))
}

// ------------------------------------------------------------------------------------- ranges, walkers

fn range_new(vm: &mut Vm, nt: &Value, _args: &[Value], _f: Gc) -> R {
    let p = proto_from(vm, nt, "Range")?;
    let o = vm.alloc(Some(p), Class::Ordinary);
    let d = wrap(vm, 0);
    for name in ["startContainer", "endContainer", "commonAncestorContainer"] {
        vm.put(o, name, d.clone(), ALL);
    }
    vm.put(o, "startOffset", Value::Num(0.0), ALL);
    vm.put(o, "endOffset", Value::Num(0.0), ALL);
    Ok(Value::Obj(o))
}

fn range_new_fn(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    range_new(vm, &Value::Undefined, &[], f)
}

fn create_contextual_fragment(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let html = str_arg(vm, args, 0, "createContextualFragment")?;
    let f = fragment(vm, &html, "body");
    Ok(wrap(vm, f))
}

/// A tree walker (or node iterator): its root, what it shows (whatToShow), its filter, where it is.
fn create_tree_walker(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let root = node_arg(vm, args, 0, "createTreeWalker")?;
    let show = match arg(args, 1) {
        Value::Undefined => 0xffff_ffff,
        v => vm.to_uint32(&v)?,
    };
    let p = host_ref(vm).proto("TreeWalker").unwrap_or_else(|| vm.proto(I::ObjectProto));
    let o = vm.alloc(Some(p), Class::Ordinary);
    let rw = wrap(vm, root);
    vm.put(o, "__root", rw.clone(), 0);
    vm.put(o, "__current", rw, 0);
    vm.put(o, "whatToShow", Value::Num(show as f64), ALL);
    vm.put(o, "filter", arg(args, 2), ALL);
    Ok(Value::Obj(o))
}

fn walker_parts(vm: &mut Vm, this: &Value) -> Result<(Gc, NodeId, NodeId, u32, Value), Value> {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    let root = get(vm, *g, "__root")?;
    let cur = get(vm, *g, "__current")?;
    let show = get(vm, *g, "whatToShow")?;
    let filter = get(vm, *g, "filter")?;
    let (Some(r), Some(c)) = (node_of(vm, &root), node_of(vm, &cur)) else { return Err(vm.type_error("Illegal invocation")) };
    let show = vm.to_uint32(&show)?;
    Ok((*g, r, c, show, filter))
}

/// Whether a walker takes a node: 1 accept, 2 reject (and what's inside it, for a walker), 3 skip.
fn accept(vm: &mut Vm, n: NodeId, show: u32, filter: &Value) -> Result<u32, Value> {
    let bit = match &doc(vm).nodes[n].data {
        Data::Element(_) => 1,
        Data::Text(_) => 4,
        Data::Comment(_) => 128,
        Data::Document => 256,
        Data::Fragment => 1024,
    };
    if show & bit == 0 {
        return Ok(3);
    }
    let w = wrap(vm, n);
    let r = if vm.is_callable(filter) {
        vm.call(filter, Value::Undefined, &[w])?
    } else if let Value::Obj(fo) = filter {
        let m = get(vm, *fo, "acceptNode")?;
        if vm.is_callable(&m) { vm.call(&m, filter.clone(), &[w])? } else { Value::Num(1.0) }
    } else {
        Value::Num(1.0)
    };
    Ok(vm.to_uint32(&r)?.clamp(1, 3))
}

fn walker_move(vm: &mut Vm, this: &Value, forward: bool) -> R {
    let (g, root, cur, show, filter) = walker_parts(vm, this)?;
    let order: Vec<NodeId> = doc(vm).descendants(root).collect();
    let at = order.iter().position(|&n| n == cur).unwrap_or(0);
    let candidates: Vec<NodeId> = if forward { order[at + 1..].to_vec() } else { order[..at].iter().rev().copied().collect() };
    for n in candidates {
        if accept(vm, n, show, &filter)? == 1 {
            let w = wrap(vm, n);
            vm.put(g, "__current", w.clone(), 0);
            return Ok(w);
        }
    }
    Ok(Value::Null)
}

fn walker_next(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    walker_move(vm, this, true)
}

fn walker_previous(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    walker_move(vm, this, false)
}

fn walker_parent(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (g, root, cur, show, filter) = walker_parts(vm, this)?;
    let mut at = doc(vm).parent(cur);
    while let Some(p) = at {
        if !doc(vm).contains(root, p) {
            break;
        }
        if accept(vm, p, show, &filter)? == 1 {
            let w = wrap(vm, p);
            vm.put(g, "__current", w.clone(), 0);
            return Ok(w);
        }
        at = doc(vm).parent(p);
    }
    Ok(Value::Null)
}

fn walker_first_child(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (g, _, cur, show, filter) = walker_parts(vm, this)?;
    let kids = doc(vm).children(cur).to_vec();
    for k in kids {
        if accept(vm, k, show, &filter)? == 1 {
            let w = wrap(vm, k);
            vm.put(g, "__current", w.clone(), 0);
            return Ok(w);
        }
    }
    Ok(Value::Null)
}

fn walker_next_sibling(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let (g, _, cur, show, filter) = walker_parts(vm, this)?;
    let mut at = doc(vm).next_sibling(cur);
    while let Some(s) = at {
        if accept(vm, s, show, &filter)? == 1 {
            let w = wrap(vm, s);
            vm.put(g, "__current", w.clone(), 0);
            return Ok(w);
        }
        at = doc(vm).next_sibling(s);
    }
    Ok(Value::Null)
}

fn walker_current(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    get(vm, *g, "__current")
}

fn walker_set_current(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    node_arg(vm, args, 0, "currentNode")?;
    vm.put(*g, "__current", arg(args, 0), 0);
    Ok(Value::Undefined)
}

fn walker_root(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let Value::Obj(g) = this else { return Err(vm.type_error("Illegal invocation")) };
    get(vm, *g, "__root")
}

// ------------------------------------------------------------------------------- elements' own members

fn element_specifics(vm: &mut Vm) {
    let p = |vm: &Vm, n: &str| host_ref(vm).proto(n).expect("made");
    // <a> and <area>: their address's parts.
    for iface in ["HTMLAnchorElement", "HTMLAreaElement"] {
        let ap = p(vm, iface);
        attribute(vm, ap, "href", anchor_href, Some(set_anchor_href));
        for part in ["protocol", "host", "hostname", "port", "pathname", "search", "hash", "origin", "username", "password"] {
            attribute_with(vm, ap, part, anchor_part, Some(set_anchor_part), js(part));
        }
        method(vm, ap, "toString", 0, anchor_href);
        attribute(vm, ap, "text", text_content, Some(set_text_content));
        attribute(vm, ap, "relList", super::lists::rel_list, None);
    }
    // Images.
    let ip = p(vm, "HTMLImageElement");
    attribute(vm, ip, "width", image_width, Some(set_image_width));
    attribute(vm, ip, "height", image_height, Some(set_image_height));
    attribute(vm, ip, "naturalWidth", natural_width, None);
    attribute(vm, ip, "naturalHeight", natural_height, None);
    attribute(vm, ip, "complete", image_complete, None);
    attribute(vm, ip, "currentSrc", image_current_src, None);
    method(vm, ip, "decode", 0, resolved_fn);
    // Form controls.
    for iface in ["HTMLInputElement", "HTMLTextAreaElement", "HTMLSelectElement", "HTMLButtonElement", "HTMLOutputElement", "HTMLFieldSetElement", "HTMLObjectElement"] {
        let cp = p(vm, iface);
        attribute(vm, cp, "form", control_form, None);
        attribute(vm, cp, "willValidate", true_fn, None);
        attribute(vm, cp, "validity", validity, None);
        attribute(vm, cp, "validationMessage", empty_string, None);
        attribute(vm, cp, "labels", labels, None);
        method(vm, cp, "checkValidity", 0, true_fn);
        method(vm, cp, "reportValidity", 0, true_fn);
        method(vm, cp, "setCustomValidity", 1, ignore_fn);
    }
    let inp = p(vm, "HTMLInputElement");
    attribute(vm, inp, "value", input_value, Some(set_input_value));
    attribute(vm, inp, "checked", input_checked, Some(set_input_checked));
    attribute(vm, inp, "type", input_type, Some(set_input_type));
    attribute(vm, inp, "valueAsNumber", value_as_number, Some(set_value_as_number));
    attribute(vm, inp, "indeterminate", false_fn, Some(ignore_fn));
    attribute(vm, inp, "selectionStart", selection_start, Some(ignore_fn));
    attribute(vm, inp, "selectionEnd", selection_end, Some(ignore_fn));
    attribute(vm, inp, "selectionDirection", none_string, Some(ignore_fn));
    attribute(vm, inp, "files", null_fn, None);
    attribute(vm, inp, "list", null_fn, None);
    method(vm, inp, "select", 0, ignore_fn);
    method(vm, inp, "setSelectionRange", 2, ignore_fn);
    method(vm, inp, "setRangeText", 1, ignore_fn);
    method(vm, inp, "stepUp", 0, ignore_fn);
    method(vm, inp, "stepDown", 0, ignore_fn);
    method(vm, inp, "showPicker", 0, ignore_fn);
    let ta = p(vm, "HTMLTextAreaElement");
    attribute(vm, ta, "value", textarea_value, Some(set_textarea_value));
    attribute(vm, ta, "defaultValue", text_content, Some(set_text_content));
    attribute(vm, ta, "type", textarea_type, None);
    attribute(vm, ta, "textLength", textarea_length, None);
    attribute(vm, ta, "selectionStart", selection_start, Some(ignore_fn));
    attribute(vm, ta, "selectionEnd", selection_end, Some(ignore_fn));
    method(vm, ta, "select", 0, ignore_fn);
    method(vm, ta, "setSelectionRange", 2, ignore_fn);
    method(vm, ta, "setRangeText", 1, ignore_fn);
    let sel = p(vm, "HTMLSelectElement");
    attribute(vm, sel, "value", select_value, Some(set_select_value));
    attribute(vm, sel, "selectedIndex", selected_index, Some(set_selected_index));
    attribute(vm, sel, "options", select_options, None);
    attribute(vm, sel, "selectedOptions", selected_options, None);
    attribute(vm, sel, "length", select_length, None);
    attribute(vm, sel, "type", select_type, None);
    method(vm, sel, "item", 1, select_item);
    method(vm, sel, "add", 1, select_add);
    method(vm, sel, "remove", 0, select_remove);
    let opt = p(vm, "HTMLOptionElement");
    attribute(vm, opt, "value", option_value, Some(set_option_value));
    attribute(vm, opt, "text", option_text, Some(set_text_content));
    attribute(vm, opt, "label", option_text, Some(ignore_fn));
    attribute(vm, opt, "selected", option_selected, Some(set_option_selected));
    attribute(vm, opt, "index", option_index, None);
    attribute(vm, opt, "form", control_form, None);
    let btn = p(vm, "HTMLButtonElement");
    attribute(vm, btn, "type", button_type, Some(set_button_type));
    let form = p(vm, "HTMLFormElement");
    attribute(vm, form, "action", form_action, Some(set_form_action));
    attribute(vm, form, "method", form_method, Some(set_form_method));
    attribute(vm, form, "elements", form_elements, None);
    attribute(vm, form, "length", form_length, None);
    method(vm, form, "submit", 0, form_submit);
    method(vm, form, "requestSubmit", 0, form_request_submit);
    method(vm, form, "reset", 0, form_reset);
    method(vm, form, "checkValidity", 0, true_fn);
    method(vm, form, "reportValidity", 0, true_fn);
    let label = p(vm, "HTMLLabelElement");
    attribute(vm, label, "control", label_control, None);
    attribute(vm, label, "form", control_form, None);
    // Scripts.
    let sp = p(vm, "HTMLScriptElement");
    attribute(vm, sp, "text", text_content, Some(set_script_text));
    attribute(vm, sp, "async", script_async, Some(set_script_async));
    method(vm, sp, "supports", 1, script_supports);
    if let Some(Value::Obj(c)) = ctor_of(vm, sp) {
        method(vm, c, "supports", 1, script_supports);
    }
    // Templates, titles, styles, links, iframes, canvases, media, dialogs, tables.
    let tp = p(vm, "HTMLTemplateElement");
    attribute(vm, tp, "content", template_content, None);
    let tt = p(vm, "HTMLTitleElement");
    attribute(vm, tt, "text", text_content, Some(set_text_content));
    for iface in ["HTMLStyleElement", "HTMLLinkElement"] {
        let sp = p(vm, iface);
        attribute(vm, sp, "sheet", sheet, None);
    }
    let lp = p(vm, "HTMLLinkElement");
    attribute(vm, lp, "relList", super::lists::rel_list, None);
    let fp = p(vm, "HTMLIFrameElement");
    attribute(vm, fp, "contentWindow", null_fn, None);
    attribute(vm, fp, "contentDocument", null_fn, None);
    // (A canvas's drawing: canvas.rs.)
    let cp = p(vm, "HTMLCanvasElement");
    method(vm, cp, "captureStream", 0, null_fn);
    method(vm, cp, "transferControlToOffscreen", 0, null_fn);
    let mp = p(vm, "HTMLMediaElement");
    for (name, v) in [("currentTime", Value::Num(0.0)), ("duration", Value::Num(f64::NAN)), ("paused", Value::Bool(true)), ("ended", Value::Bool(false)), ("muted", Value::Bool(false)), ("volume", Value::Num(1.0)), ("playbackRate", Value::Num(1.0)), ("defaultPlaybackRate", Value::Num(1.0)), ("readyState", Value::Num(0.0)), ("networkState", Value::Num(0.0)), ("seeking", Value::Bool(false)), ("currentSrc", js("")), ("error", Value::Null), ("buffered", Value::Null), ("played", Value::Null), ("seekable", Value::Null), ("textTracks", Value::Null), ("srcObject", Value::Null)] {
        vm.put(mp, name, v, ALL);
    }
    method(vm, mp, "play", 0, rejected_fn);
    method(vm, mp, "pause", 0, ignore_fn);
    method(vm, mp, "load", 0, ignore_fn);
    method(vm, mp, "canPlayType", 1, empty_string);
    method(vm, mp, "addTextTrack", 1, null_fn);
    let vp = p(vm, "HTMLVideoElement");
    vm.put(vp, "videoWidth", Value::Num(0.0), ALL);
    vm.put(vp, "videoHeight", Value::Num(0.0), ALL);
    method(vm, vp, "requestPictureInPicture", 0, rejected_fn);
    method(vm, vp, "getVideoPlaybackQuality", 0, null_fn);
    let dp = p(vm, "HTMLDialogElement");
    method(vm, dp, "show", 0, dialog_show);
    method(vm, dp, "showModal", 0, dialog_show);
    method(vm, dp, "close", 0, dialog_close);
    vm.put(dp, "returnValue", js(""), ALL);
    let tbl = p(vm, "HTMLTableElement");
    attribute(vm, tbl, "rows", table_rows, None);
    attribute(vm, tbl, "tBodies", table_bodies, None);
    attribute(vm, tbl, "tHead", table_head, None);
    attribute(vm, tbl, "tFoot", table_foot, None);
    attribute(vm, tbl, "caption", table_caption, None);
    method(vm, tbl, "insertRow", 0, insert_row);
    method(vm, tbl, "deleteRow", 1, delete_row);
    method(vm, tbl, "createTBody", 0, create_tbody);
    let sec = p(vm, "HTMLTableSectionElement");
    attribute(vm, sec, "rows", section_rows, None);
    method(vm, sec, "insertRow", 0, insert_row);
    method(vm, sec, "deleteRow", 1, delete_row);
    let tr = p(vm, "HTMLTableRowElement");
    attribute(vm, tr, "cells", row_cells, None);
    attribute(vm, tr, "rowIndex", row_index, None);
    attribute(vm, tr, "sectionRowIndex", row_index, None);
    method(vm, tr, "insertCell", 0, insert_cell);
    method(vm, tr, "deleteCell", 1, delete_cell);
    let td = p(vm, "HTMLTableCellElement");
    attribute(vm, td, "cellIndex", cell_index, None);
    let slot_p = p(vm, "HTMLSlotElement");
    method(vm, slot_p, "assignedNodes", 0, empty_array);
    method(vm, slot_p, "assignedElements", 0, empty_array);
    method(vm, slot_p, "assign", 0, ignore_fn);
    let prog = p(vm, "HTMLProgressElement");
    attribute(vm, prog, "value", progress_value, Some(set_progress_value));
    attribute(vm, prog, "position", progress_position, None);
    let body = p(vm, "HTMLBodyElement");
    // (The body's onload and the like are the window's.)
    for name in ["load", "unload", "beforeunload", "hashchange", "popstate", "message", "online", "offline", "pagehide", "pageshow", "resize", "storage"] {
        attribute_with(vm, body, &format!("on{name}"), body_handler, Some(set_body_handler), js(name));
    }
}

fn resolved_fn(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let p = vm.new_promise();
    vm.fulfill_promise(p, Value::Undefined);
    Ok(Value::Obj(p))
}

fn none_string(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("none"))
}

fn anchor_url(vm: &mut Vm, n: NodeId) -> Option<url::Url> {
    let h = attr(vm, n, "href")?;
    url::Url::parse(&h, Some(&host_ref(vm).base))
}

fn anchor_href(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(match (attr(vm, n, "href"), anchor_url(vm, n)) {
        (_, Some(u)) => js(&u.href()),
        (Some(h), None) => js(&h),
        (None, None) => js(""),
    })
}

fn set_anchor_href(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, "href", &v);
    Ok(Value::Undefined)
}

/// An address's part, as location and links give them.
pub fn url_part(u: &url::Url, part: &str) -> String {
    match part {
        "href" => u.href(),
        "protocol" => format!("{}:", u.scheme),
        "host" => u.host_port(),
        "hostname" => u.host.clone().unwrap_or_default(),
        "port" => u.port.map(|p| format!("{p}")).unwrap_or_default(),
        "pathname" => u.path.clone(),
        "search" => u.search(),
        "hash" => u.hash(),
        "origin" => u.origin(),
        "username" => u.username.clone(),
        "password" => u.password.clone(),
        _ => String::new(),
    }
}

fn anchor_part(vm: &mut Vm, this: &Value, _args: &[Value], f: Gc) -> R {
    let n = this_element(vm, this)?;
    let part = string(vm, &vm.data(f))?;
    Ok(js(&anchor_url(vm, n).map(|u| url_part(&u, &part)).unwrap_or_default()))
}

fn set_anchor_part(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let n = this_element(vm, this)?;
    let part = string(vm, &vm.data(f))?;
    let v = string(vm, &arg(args, 0))?;
    if let Some(mut u) = anchor_url(vm, n)
        && u.set(&part, &v)
    {
        set_attr(vm, n, "href", &u.href());
    }
    Ok(Value::Undefined)
}

/// An <img>'s width or height: its attribute's, else as it's drawn, else its picture's own.
fn image_size(vm: &mut Vm, this: &Value, height: bool) -> R {
    let n = this_element(vm, this)?;
    if let Some(v) = attr(vm, n, if height { "height" } else { "width" }).and_then(|v| parse_int(&v)) {
        return Ok(Value::Num(v.max(0) as f64));
    }
    if let Some(r) = rect(vm, n).filter(|r| r.2 > 0.0 || r.3 > 0.0) {
        return Ok(Value::Num(if height { r.3 } else { r.2 } as f64));
    }
    natural(vm, n, height)
}

/// An <img>'s picture's own size, once it has come (0 till then).
fn natural(vm: &mut Vm, n: NodeId, height: bool) -> R {
    let size = host_ref(vm).settled.get(&n).copied().flatten();
    Ok(Value::Num(size.map_or(0, |(w, h)| if height { h } else { w }) as f64))
}

fn natural_width(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    natural(vm, n, false)
}

fn natural_height(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    natural(vm, n, true)
}

/// Whether an <img> is done: it has no picture to show, or its picture has come (or couldn't be had).
fn image_complete(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let src = attr(vm, n, "src").filter(|s| !s.trim().is_empty()).or_else(|| attr(vm, n, "srcset").filter(|s| !s.trim().is_empty()));
    Ok(Value::Bool(src.is_none() || host_ref(vm).settled.contains_key(&n)))
}

fn image_width(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    image_size(vm, this, false)
}

fn image_height(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    image_size(vm, this, true)
}

fn set_image_width(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = vm.to_uint32(&arg(args, 0))?;
    set_attr(vm, n, "width", &format!("{v}"));
    Ok(Value::Undefined)
}

fn set_image_height(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = vm.to_uint32(&arg(args, 0))?;
    set_attr(vm, n, "height", &format!("{v}"));
    Ok(Value::Undefined)
}

fn image_current_src(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(&attr(vm, n, "src").map(|s| resolve(vm, &s)).unwrap_or_default()))
}

fn image_new(vm: &mut Vm, _nt: &Value, args: &[Value], _f: Gc) -> R {
    let w = create(vm, "img")?;
    if let Some(n) = node_of(vm, &w) {
        if !arg(args, 0).is_undefined() {
            let v = vm.to_uint32(&arg(args, 0))?;
            set_attr(vm, n, "width", &format!("{v}"));
        }
        if !arg(args, 1).is_undefined() {
            let v = vm.to_uint32(&arg(args, 1))?;
            set_attr(vm, n, "height", &format!("{v}"));
        }
    }
    Ok(w)
}

fn option_new(vm: &mut Vm, _nt: &Value, args: &[Value], _f: Gc) -> R {
    let w = create(vm, "option")?;
    if let Some(n) = node_of(vm, &w) {
        if !arg(args, 0).is_undefined() {
            let t = string(vm, &arg(args, 0))?;
            let tn = host(vm).doc.create(Data::Text(t));
            host(vm).doc.insert(n, tn, None);
        }
        if !arg(args, 1).is_undefined() {
            let v = string(vm, &arg(args, 1))?;
            set_attr(vm, n, "value", &v);
        }
        if arg(args, 3).truthy() {
            set_attr(vm, n, "selected", "");
        }
    }
    Ok(w)
}

fn control_form(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let f = form_of(vm, n);
    Ok(wrap_opt(vm, f))
}

fn validity(vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    let o = vm.new_object();
    for name in ["badInput", "customError", "patternMismatch", "rangeOverflow", "rangeUnderflow", "stepMismatch", "tooLong", "tooShort", "typeMismatch", "valueMissing"] {
        vm.put(o, name, Value::Bool(false), ALL);
    }
    vm.put(o, "valid", Value::Bool(true), ALL);
    Ok(Value::Obj(o))
}

fn labels(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let id = attr(vm, n, "id");
    let d = doc(vm);
    let found: Vec<NodeId> = d.descendants(0).filter(|&l| d.name(l) == "label" && (id.is_some() && d.element(l).and_then(|e| e.attr("for")) == id.as_deref() || d.contains(l, n))).collect();
    Ok(node_list(vm, found))
}

fn input_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let ty = attr(vm, n, "type").unwrap_or_default().to_ascii_lowercase();
    Ok(js(&match attr(vm, n, "value") {
        Some(v) => v,
        None if matches!(ty.as_str(), "checkbox" | "radio") => String::from("on"),
        None => String::new(),
    }))
}

fn set_input_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = arg(args, 0);
    let s = if matches!(v, Value::Null) { String::new() } else { string(vm, &v)? };
    set_attr(vm, n, "value", &s);
    Ok(Value::Undefined)
}

fn input_checked(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Bool(attr(vm, n, "checked").is_some()))
}

fn set_input_checked(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if arg(args, 0).truthy() {
        if attr(vm, n, "type").is_some_and(|t| t.eq_ignore_ascii_case("radio")) {
            uncheck_group(vm, n);
        }
        set_attr(vm, n, "checked", "");
    } else {
        remove_attr(vm, n, "checked");
    }
    Ok(Value::Undefined)
}

const INPUT_TYPES: &[&str] = &[
    "hidden", "text", "search", "tel", "url", "email", "password", "date", "month", "week", "time", "datetime-local",
    "number", "range", "color", "checkbox", "radio", "file", "submit", "image", "reset", "button",
];

fn input_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let t = attr(vm, n, "type").unwrap_or_default().to_ascii_lowercase();
    Ok(js(if INPUT_TYPES.contains(&t.as_str()) { &t } else { "text" }))
}

fn set_input_type(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, "type", &v);
    Ok(Value::Undefined)
}

fn value_as_number(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let v = input_value(vm, this, args, f)?;
    let s = string(vm, &v)?;
    Ok(Value::Num(s.trim().parse::<f64>().unwrap_or(f64::NAN)))
}

fn set_value_as_number(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let x = vm.to_number(&arg(args, 0))?;
    let s = string(vm, &Value::Num(x))?;
    set_attr(vm, n, "value", &s);
    Ok(Value::Undefined)
}

fn selection_start(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let len = attr(vm, n, "value").map_or(0, |v| v.encode_utf16().count());
    Ok(Value::Num(len as f64))
}

fn selection_end(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    selection_start(vm, this, args, f)
}

/// A text area's value (what's typed in it is its text).
fn textarea_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(&match attr(vm, n, "value") {
        Some(v) => v,
        None => doc(vm).text_of(n),
    }))
}

fn set_textarea_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = arg(args, 0);
    let s = if matches!(v, Value::Null) { String::new() } else { string(vm, &v)? };
    set_attr(vm, n, "value", &s);
    Ok(Value::Undefined)
}

fn textarea_type(_vm: &mut Vm, _this: &Value, _args: &[Value], _f: Gc) -> R {
    Ok(js("textarea"))
}

fn textarea_length(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let v = textarea_value(vm, this, args, f)?;
    let s = string(vm, &v)?;
    Ok(Value::Num(s.encode_utf16().count() as f64))
}

/// A <select>'s options, in order.
fn options_of(d: &Document, n: NodeId) -> Vec<NodeId> {
    d.descendants(n).filter(|&o| d.name(o) == "option").collect()
}

fn selected_of(d: &Document, n: NodeId) -> Option<NodeId> {
    let opts = options_of(d, n);
    opts.iter().copied().find(|&o| d.element(o).is_some_and(|e| e.has_attr("selected"))).or_else(|| opts.first().copied())
}

fn option_value_of(d: &Document, o: NodeId) -> String {
    match d.element(o).and_then(|e| e.attr("value")) {
        Some(v) => String::from(v),
        None => d.text_of(o).split_whitespace().collect::<Vec<&str>>().join(" "),
    }
}

fn select_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    Ok(js(&selected_of(d, n).map(|o| option_value_of(d, o)).unwrap_or_default()))
}

fn select_option(vm: &mut Vm, n: NodeId, pick: Option<NodeId>) {
    let opts = options_of(doc(vm), n);
    for o in opts {
        if Some(o) == pick {
            set_attr(vm, o, "selected", "");
        } else {
            remove_attr(vm, o, "selected");
        }
    }
}

fn set_select_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    let d = doc(vm);
    let pick = options_of(d, n).into_iter().find(|&o| option_value_of(d, o) == v);
    select_option(vm, n, pick);
    Ok(Value::Undefined)
}

fn selected_index(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let opts = options_of(d, n);
    Ok(Value::Num(match selected_of(d, n) {
        Some(s) => opts.iter().position(|&o| o == s).map_or(-1.0, |i| i as f64),
        None => -1.0,
    }))
}

fn set_selected_index(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let i = vm.to_int32(&arg(args, 0))?;
    let opts = options_of(doc(vm), n);
    let pick = if i >= 0 { opts.get(i as usize).copied() } else { None };
    select_option(vm, n, pick);
    Ok(Value::Undefined)
}

fn select_options(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(super::lists::live(vm, n, ListKind::Options))
}

fn selected_options(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let s = selected_of(doc(vm), n);
    Ok(node_list(vm, s.into_iter().collect()))
}

fn select_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(options_of(doc(vm), n).len() as f64))
}

fn select_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(if attr(vm, n, "multiple").is_some() { "select-multiple" } else { "select-one" }))
}

fn select_item(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let i = vm.to_uint32(&arg(args, 0))? as usize;
    let o = options_of(doc(vm), n).get(i).copied();
    Ok(wrap_opt(vm, o))
}

fn select_add(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let o = node_arg(vm, args, 0, "add")?;
    let before = match arg(args, 1) {
        Value::Num(i) => options_of(doc(vm), n).get(i as usize).copied(),
        v => node_of(vm, &v),
    };
    let parent = before.and_then(|b| doc(vm).parent(b)).unwrap_or(n);
    pre_insert(vm, parent, o, before)?;
    Ok(Value::Undefined)
}

fn select_remove(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if args.is_empty() {
        remove(vm, n);
        return Ok(Value::Undefined);
    }
    let i = vm.to_int32(&arg(args, 0))?;
    if i >= 0
        && let Some(o) = options_of(doc(vm), n).get(i as usize).copied()
    {
        remove(vm, o);
    }
    Ok(Value::Undefined)
}

fn option_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(&option_value_of(doc(vm), n)))
}

fn set_option_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, "value", &v);
    Ok(Value::Undefined)
}

fn option_text(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(&doc(vm).text_of(n).split_whitespace().collect::<Vec<&str>>().join(" ")))
}

fn option_selected(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let sel = d.ancestor_named(n, "select");
    Ok(Value::Bool(match sel {
        Some(s) => selected_of(d, s) == Some(n),
        None => d.element(n).is_some_and(|e| e.has_attr("selected")),
    }))
}

fn set_option_selected(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let sel = doc(vm).ancestor_named(n, "select");
    if arg(args, 0).truthy() {
        match sel {
            Some(s) if attr(vm, s, "multiple").is_none() => select_option(vm, s, Some(n)),
            _ => set_attr(vm, n, "selected", ""),
        }
    } else {
        remove_attr(vm, n, "selected");
    }
    Ok(Value::Undefined)
}

fn option_index(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    Ok(Value::Num(d.ancestor_named(n, "select").and_then(|s| options_of(d, s).iter().position(|&o| o == n)).unwrap_or(0) as f64))
}

fn button_type(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let t = attr(vm, n, "type").unwrap_or_default().to_ascii_lowercase();
    Ok(js(if matches!(t.as_str(), "button" | "reset") { &t } else { "submit" }))
}

fn set_button_type(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, "type", &v);
    Ok(Value::Undefined)
}

fn form_action(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(js(&match attr(vm, n, "action") {
        Some(a) if !a.is_empty() => resolve(vm, &a),
        _ => host_ref(vm).url.href(),
    }))
}

fn set_form_action(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, "action", &v);
    Ok(Value::Undefined)
}

fn form_method(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let m = attr(vm, n, "method").unwrap_or_default().to_ascii_lowercase();
    Ok(js(if m == "post" || m == "dialog" { &m } else { "get" }))
}

fn set_form_method(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let v = string(vm, &arg(args, 0))?;
    set_attr(vm, n, "method", &v);
    Ok(Value::Undefined)
}

fn form_elements(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(super::lists::live(vm, n, ListKind::Controls))
}

fn form_length(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(super::lists::controls(doc(vm), n).len() as f64))
}

fn form_submit(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    super::window::request_submit(vm, n, None);
    Ok(Value::Undefined)
}

fn form_request_submit(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let s = node_of(vm, &arg(args, 0));
    submit_form(vm, n, s);
    Ok(Value::Undefined)
}

fn form_reset(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let ok = super::events::fire(vm, this, &EventInit::new("reset", "Event", true, true));
    if ok {
        let ctrls = super::lists::controls(doc(vm), n);
        for c in ctrls {
            if doc(vm).name(c) == "textarea" {
                remove_attr(vm, c, "value");
            }
        }
    }
    Ok(Value::Undefined)
}

fn label_control(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let c = match d.element(n).and_then(|e| e.attr("for")) {
        Some(id) => d.descendants(0).find(|&e| d.element(e).and_then(|el| el.attr("id")) == Some(id)),
        None => d.descendants(n).skip(1).find(|&e| matches!(d.name(e), "input" | "select" | "textarea" | "button")),
    };
    Ok(wrap_opt(vm, c))
}

fn set_script_text(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    set_text_content(vm, this, args, f)?;
    // (A script in the page that hadn't run, given text now, runs.)
    let n = this_element(vm, this)?;
    if doc(vm).connected(n) {
        script_inserted(vm, n);
    }
    Ok(Value::Undefined)
}

fn script_async(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    // (A script made by a script is async unless it's told otherwise.)
    Ok(Value::Bool(attr(vm, n, "async").is_some() || !host_ref(vm).started.contains(&n)))
}

fn set_script_async(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if arg(args, 0).truthy() {
        set_attr(vm, n, "async", "");
    } else {
        remove_attr(vm, n, "async");
    }
    Ok(Value::Undefined)
}

fn script_supports(vm: &mut Vm, _this: &Value, args: &[Value], _f: Gc) -> R {
    let t = string(vm, &arg(args, 0))?;
    Ok(Value::Bool(t == "classic"))
}

fn template_content(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let f = host(vm).doc.template_content(n);
    Ok(wrap(vm, f))
}

fn sheet(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let o = vm.new_object();
    let w = wrap(vm, n);
    vm.put(o, "ownerNode", w, ALL);
    let rules = vm.new_array(Vec::new());
    vm.put(o, "cssRules", Value::Obj(rules), ALL);
    vm.put(o, "disabled", Value::Bool(false), ALL);
    method(vm, o, "insertRule", 2, sheet_insert_rule);
    method(vm, o, "deleteRule", 1, ignore_fn);
    Ok(Value::Obj(o))
}

/// sheet.insertRule: the rule added to its <style>'s text (so the page's styles have it).
fn sheet_insert_rule(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let rule = string(vm, &arg(args, 0))?;
    if let Value::Obj(o) = this {
        let owner = get(vm, *o, "ownerNode")?;
        if let Some(n) = node_of(vm, &owner)
            && doc(vm).name(n) == "style"
        {
            let mut t = doc(vm).text_of(n);
            t.push('\n');
            t.push_str(&rule);
            let tn = host(vm).doc.create(Data::Text(t));
            replace_all(vm, n, Some(tn));
            host(vm).rescan = true;
        }
    }
    Ok(Value::Num(0.0))
}

fn dialog_show(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    set_attr(vm, n, "open", "");
    Ok(Value::Undefined)
}

fn dialog_close(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    if !arg(args, 0).is_undefined()
        && let Value::Obj(g) = this
    {
        let v = string(vm, &arg(args, 0))?;
        vm.put(*g, "returnValue", js(&v), ALL);
    }
    remove_attr(vm, n, "open");
    super::events::fire(vm, this, &EventInit::new("close", "Event", false, false));
    Ok(Value::Undefined)
}

fn table_rows(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let mut rows = Vec::new();
    for c in element_children(d, n) {
        match d.name(c) {
            "tr" => rows.push(c),
            "thead" | "tbody" | "tfoot" => rows.extend(element_children(d, c).into_iter().filter(|&r| d.name(r) == "tr")),
            _ => {}
        }
    }
    Ok(node_list(vm, rows))
}

fn table_bodies(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let b: Vec<NodeId> = element_children(d, n).into_iter().filter(|&c| d.name(c) == "tbody").collect();
    Ok(node_list(vm, b))
}

fn table_child(vm: &mut Vm, this: &Value, name: &str) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let c = element_children(d, n).into_iter().find(|&c| d.name(c) == name);
    Ok(wrap_opt(vm, c))
}

fn table_head(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    table_child(vm, this, "thead")
}
fn table_foot(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    table_child(vm, this, "tfoot")
}
fn table_caption(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    table_child(vm, this, "caption")
}

fn section_rows(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let rows: Vec<NodeId> = element_children(d, n).into_iter().filter(|&c| d.name(c) == "tr").collect();
    Ok(node_list(vm, rows))
}

fn new_element(vm: &mut Vm, name: &str) -> NodeId {
    host(vm).doc.create(Data::Element(Element { name: String::from(name), attrs: Vec::new() }))
}

fn insert_row(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let i = if arg(args, 0).is_undefined() { -1 } else { vm.to_int32(&arg(args, 0))? };
    let d = doc(vm);
    // (A table without sections gets its rows in its last tbody, or a new one.)
    let section = if d.name(n) == "table" {
        match element_children(d, n).into_iter().filter(|&c| d.name(c) == "tbody").last() {
            Some(b) => b,
            None => {
                let b = new_element(vm, "tbody");
                pre_insert(vm, n, b, None)?;
                b
            }
        }
    } else {
        n
    };
    let rows: Vec<NodeId> = element_children(doc(vm), section).into_iter().filter(|&c| doc(vm).name(c) == "tr").collect();
    if i > rows.len() as i32 || i < -1 {
        return Err(dom_exception(vm, "IndexSizeError", "The index provided is outside the table's rows."));
    }
    let before = if i < 0 { None } else { rows.get(i as usize).copied() };
    let r = new_element(vm, "tr");
    pre_insert(vm, section, r, before)?;
    Ok(wrap(vm, r))
}

fn delete_row(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let n = this_element(vm, this)?;
    let rows = if doc(vm).name(n) == "table" { table_rows(vm, this, &[], f)? } else { section_rows(vm, this, &[], f)? };
    let items = vm.array_values(&rows);
    let i = vm.to_int32(&arg(args, 0))?;
    let at = if i == -1 { items.len().checked_sub(1) } else { Some(i as usize) };
    if let Some(r) = at.and_then(|k| items.get(k)).and_then(|v| node_of(vm, v)) {
        remove(vm, r);
    }
    Ok(Value::Undefined)
}

fn create_tbody(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let b = new_element(vm, "tbody");
    pre_insert(vm, n, b, None)?;
    Ok(wrap(vm, b))
}

fn row_cells(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let cells: Vec<NodeId> = element_children(d, n).into_iter().filter(|&c| matches!(d.name(c), "td" | "th")).collect();
    Ok(node_list(vm, cells))
}

fn row_index(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let Some(p) = d.parent(n) else { return Ok(Value::Num(-1.0)) };
    let rows: Vec<NodeId> = element_children(d, p).into_iter().filter(|&c| d.name(c) == "tr").collect();
    Ok(Value::Num(rows.iter().position(|&r| r == n).map_or(-1.0, |i| i as f64)))
}

fn insert_cell(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let i = if arg(args, 0).is_undefined() { -1 } else { vm.to_int32(&arg(args, 0))? };
    let cells: Vec<NodeId> = element_children(doc(vm), n).into_iter().filter(|&c| matches!(doc(vm).name(c), "td" | "th")).collect();
    let before = if i < 0 { None } else { cells.get(i as usize).copied() };
    let c = new_element(vm, "td");
    pre_insert(vm, n, c, before)?;
    Ok(wrap(vm, c))
}

fn delete_cell(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let i = vm.to_int32(&arg(args, 0))?;
    let cells: Vec<NodeId> = element_children(doc(vm), n).into_iter().filter(|&c| matches!(doc(vm).name(c), "td" | "th")).collect();
    let at = if i == -1 { cells.len().checked_sub(1) } else { Some(i as usize) };
    if let Some(c) = at.and_then(|k| cells.get(k).copied()) {
        remove(vm, c);
    }
    Ok(Value::Undefined)
}

fn cell_index(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let d = doc(vm);
    let Some(p) = d.parent(n) else { return Ok(Value::Num(-1.0)) };
    let cells: Vec<NodeId> = element_children(d, p).into_iter().filter(|&c| matches!(d.name(c), "td" | "th")).collect();
    Ok(Value::Num(cells.iter().position(|&c| c == n).map_or(-1.0, |i| i as f64)))
}

fn progress_value(vm: &mut Vm, this: &Value, _args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    Ok(Value::Num(attr(vm, n, "value").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0)))
}

fn set_progress_value(vm: &mut Vm, this: &Value, args: &[Value], _f: Gc) -> R {
    let n = this_element(vm, this)?;
    let x = vm.to_number(&arg(args, 0))?;
    let s = string(vm, &Value::Num(x))?;
    set_attr(vm, n, "value", &s);
    Ok(Value::Undefined)
}

fn progress_position(vm: &mut Vm, this: &Value, args: &[Value], f: Gc) -> R {
    let n = this_element(vm, this)?;
    if attr(vm, n, "value").is_none() {
        return Ok(Value::Num(-1.0));
    }
    let v = progress_value(vm, this, args, f)?;
    let max = attr(vm, n, "max").and_then(|m| m.trim().parse::<f64>().ok()).filter(|m| *m > 0.0).unwrap_or(1.0);
    Ok(Value::Num(match v {
        Value::Num(x) => (x / max).clamp(0.0, 1.0),
        _ => 0.0,
    }))
}

fn body_handler(vm: &mut Vm, _this: &Value, _args: &[Value], f: Gc) -> R {
    let ty = string(vm, &vm.data(f))?;
    let w = host_ref(vm).window;
    get(vm, w, &format!("on{ty}"))
}

fn set_body_handler(vm: &mut Vm, _this: &Value, args: &[Value], f: Gc) -> R {
    let ty = string(vm, &vm.data(f))?;
    let w = host_ref(vm).window;
    super::events::set_handler(vm, w, &ty, arg(args, 0));
    Ok(Value::Undefined)
}
