use std::collections::HashSet;

use crate::document::Document;

const MAX_ITEMS: usize = 60;
const SCAN_LINES: usize = 2500;
const MAX_SCAN_LINE_BYTES: usize = 4000;

const PHP_KEYWORDS: &str = "abstract array as break callable case catch class clone const continue declare default do echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile enum extends false final finally fn for foreach function global if implements include include_once instanceof insteadof interface isset list match namespace new null print private protected public readonly require require_once return self static switch throw trait true try unset use var while yield parent bool int float string iterable object mixed void never";

const PHP_VARS: &str =
    "$this $_GET $_POST $_REQUEST $_SERVER $_SESSION $_COOKIE $_FILES $_ENV $GLOBALS $argv $argc";

const PHP_FUNCS: &str = "abs addslashes array_chunk array_column array_combine array_count_values array_diff array_diff_assoc array_diff_key array_fill array_fill_keys array_filter array_flip array_intersect array_intersect_key array_is_list array_key_exists array_key_first array_key_last array_keys array_map array_merge array_merge_recursive array_multisort array_pad array_pop array_product array_push array_rand array_reduce array_replace array_reverse array_search array_shift array_slice array_splice array_sum array_unique array_unshift array_values array_walk arsort asort base64_decode base64_encode basename bin2hex boolval call_user_func call_user_func_array ceil checkdate chmod chr chunk_split class_exists compact constant copy count crc32 ctype_alnum ctype_alpha ctype_digit curl_close curl_error curl_exec curl_getinfo curl_init curl_setopt curl_setopt_array current date date_create date_default_timezone_set date_diff date_format debug_backtrace dechex define defined dirname explode extract fclose feof fgetcsv fgets file file_exists file_get_contents file_put_contents filemtime filesize filter_input filter_var floatval floor fopen fputcsv fputs fread fseek func_get_args function_exists fwrite get_class get_object_vars getenv gettype glob gmdate hash hash_hmac header headers_sent hex2bin html_entity_decode htmlentities htmlspecialchars htmlspecialchars_decode http_build_query iconv implode in_array ini_get ini_set intdiv interface_exists intval is_array is_bool is_callable is_dir is_file is_float is_int is_null is_numeric is_object is_readable is_string is_writable iterator_to_array json_decode json_encode json_last_error json_last_error_msg key krsort ksort lcfirst ltrim mail max mb_convert_case mb_convert_encoding mb_internal_encoding mb_strlen mb_strpos mb_strrpos mb_strtolower mb_strtoupper mb_substr mb_substr_count md5 md5_file method_exists microtime min mkdir mktime move_uploaded_file nl2br number_format ob_end_clean ob_get_clean ob_get_contents ob_start opendir ord parse_str parse_url password_hash password_verify pathinfo pow preg_grep preg_match preg_match_all preg_quote preg_replace preg_replace_callback preg_split print_r printf property_exists random_bytes random_int range rawurldecode rawurlencode readdir realpath rename reset rmdir round rsort rtrim scandir serialize session_destroy session_id session_start set_error_handler set_exception_handler set_time_limit setcookie settype sha1 shuffle simplexml_load_file simplexml_load_string sleep sort sprintf sqrt str_contains str_ends_with str_ireplace str_pad str_repeat str_replace str_split str_starts_with str_word_count strcasecmp strcmp strip_tags stripos stripslashes stristr strlen strncmp strpos strrchr strrev strrpos strstr strtolower strtotime strtoupper strtr strval substr substr_count substr_replace sys_get_temp_dir tempnam time touch trigger_error trim uasort ucfirst ucwords uksort uniqid unlink unserialize urldecode urlencode usleep usort var_dump var_export version_compare vsprintf wordwrap";

const HTML_TAGS: &str = "a abbr address area article aside audio b base blockquote body br button canvas caption cite code col colgroup data datalist dd del details dfn dialog div dl dt em embed fieldset figcaption figure footer form h1 h2 h3 h4 h5 h6 head header hr html i iframe img input ins kbd label legend li link main map mark menu meta meter nav noscript object ol optgroup option output p picture pre progress q s samp script section select slot small source span strong style sub summary sup svg table tbody td template textarea tfoot th thead time title tr track u ul var video wbr";

const HTML_ATTRS: &str = "accept action alt aria-hidden aria-label async autocomplete autofocus autoplay charset checked class colspan content controls crossorigin data-id datetime defer dir disabled download draggable enctype for form height hidden href hreflang id integrity itemprop itemscope itemtype lang loading loop max maxlength media method min minlength multiple muted name novalidate onclick onchange onsubmit pattern placeholder poster preload readonly rel required role rows rowspan selected sizes src srcset step style tabindex target title type value width";

const TWIG_WORDS: &str = "apply autoescape block do embed endapply endautoescape endblock endembed endfor endif endmacro endset endverbatim endwith extends for from if elseif else import include macro set use verbatim with abs batch capitalize column date default escape filter first format join json_encode keys last length lower map merge nl2br number_format raw reduce replace reverse round slice sort split striptags title trim upper url_encode attribute constant cycle dump parent random range source loop defined empty even odd iterable sameas";

const JS_KEYWORDS: &str = "async await break case catch class const continue debugger default delete do else export extends false finally for function if import in instanceof let new null of return static super switch this throw true try typeof undefined var void while yield";

const JS_GLOBALS: &str = "Array Boolean Date Error FormData JSON Map Math Number Object Promise Proxy RegExp Set String Symbol URL URLSearchParams WeakMap alert cancelAnimationFrame clearInterval clearTimeout confirm console decodeURIComponent document encodeURIComponent fetch history isNaN jQuery localStorage location navigator parseFloat parseInt prompt requestAnimationFrame sessionStorage setInterval setTimeout window";

const JS_METHODS: &str = "addEventListener appendChild apply assign bind call catch charAt classList closest concat contains createElement dataset entries every filter finally find findIndex flat flatMap forEach from getAttribute getElementById getElementsByClassName includes indexOf innerHTML innerText insertAdjacentHTML insertBefore isArray join keys lastIndexOf length log map match padStart parentNode parse pop preventDefault push querySelector querySelectorAll reduce remove removeAttribute removeChild removeEventListener replace replaceAll reverse setAttribute shift slice some sort splice split startsWith stopPropagation stringify style substring test textContent then toFixed toLowerCase toString toUpperCase trim unshift value values";

const JQUERY: &str = "addClass after ajax animate append appendTo attr before blur change children click clone closest css data delay detach each empty eq fadeIn fadeOut fadeToggle filter find first focus get getJSON has hasClass height hide hover html index insertAfter insertBefore is keydown keyup last load next nextAll not off offset on one outerHeight outerWidth parent parents position post prepend prependTo prev prevAll prop ready remove removeAttr removeClass removeData replaceWith resize scroll scrollLeft scrollTop serialize serializeArray show siblings slideDown slideToggle slideUp stop submit text toggle toggleClass trigger unwrap val width wrap wrapAll";

const CSS_PROPS: &str = "align-content align-items align-self animation animation-delay animation-duration aspect-ratio backdrop-filter background background-attachment background-clip background-color background-image background-position background-repeat background-size border border-bottom border-collapse border-color border-left border-radius border-right border-spacing border-style border-top border-width bottom box-shadow box-sizing clear clip-path color column-gap content cursor display filter flex flex-basis flex-direction flex-flow flex-grow flex-shrink flex-wrap float font font-family font-size font-style font-weight gap grid grid-area grid-column grid-row grid-template-areas grid-template-columns grid-template-rows height inset justify-content justify-items justify-self left letter-spacing line-height list-style margin margin-bottom margin-left margin-right margin-top max-height max-width min-height min-width object-fit object-position opacity order outline overflow overflow-x overflow-y padding padding-bottom padding-left padding-right padding-top pointer-events position resize right row-gap scroll-behavior text-align text-decoration text-indent text-overflow text-shadow text-transform top transform transform-origin transition user-select vertical-align visibility white-space width word-break word-wrap z-index";

const CSS_VALUES: &str = "absolute auto baseline block bold border-box both center column contain cover currentColor dashed fixed flex flex-end flex-start grid hidden inherit initial inline inline-block inline-flex italic none normal nowrap pointer relative repeat row scroll solid space-around space-between static sticky stretch transparent underline uppercase visible wrap important";

const PYTHON: &str = "and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield abs all any bool bytes callable chr classmethod dict dir divmod enumerate eval filter float format frozenset getattr hasattr hash id input int isinstance issubclass iter len list map max min next object open ord pow print property range repr reversed round set setattr slice sorted staticmethod str sum super tuple type vars zip self __init__ __name__ __main__ Exception ValueError TypeError KeyError IndexError";

const RUST: &str = "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while bool char f32 f64 i8 i16 i32 i64 i128 isize str u8 u16 u32 u64 u128 usize Box Option Result Some None Ok Err String Vec HashMap HashSet Rc Arc RefCell Mutex Clone Copy Debug Default Eq PartialEq Ord PartialOrd Hash Iterator From Into println eprintln format vec assert assert_eq unwrap expect collect iter into_iter map filter push len is_empty to_string to_owned clone as_str";

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Php,
    Html,
    Twig,
    Js,
    Css,
    Python,
    Rust,
    Other,
}

fn lang(syntax: &str) -> Lang {
    let s = syntax.to_lowercase();
    if s.contains("twig") {
        Lang::Twig
    } else if s.starts_with("php") {
        Lang::Php
    } else if s.starts_with("html") || s == "vue component" || s == "xml" {
        Lang::Html
    } else if s.starts_with("javascript") || s.starts_with("typescript") || s == "json" {
        Lang::Js
    } else if matches!(s.as_str(), "css" | "scss" | "sass" | "less") {
        Lang::Css
    } else if s == "python" {
        Lang::Python
    } else if s == "rust" {
        Lang::Rust
    } else {
        Lang::Other
    }
}

fn is_word(c: char, lang: Lang) -> bool {
    c.is_alphanumeric()
        || c == '_'
        || (c == '$' && matches!(lang, Lang::Php | Lang::Js))
        || (c == '-' && lang == Lang::Css)
}

pub struct Item {
    pub text: String,
    pub kind: &'static str,
}

#[derive(Default)]
pub struct Completion {
    pub open: bool,
    pub items: Vec<Item>,
    pub selected: usize,
    pub start: usize,
}

fn dictionaries(
    lang: Lang,
    prefix: &str,
    before: Option<char>,
) -> Vec<(&'static str, &'static str)> {
    match lang {
        Lang::Php if prefix.starts_with('$') => vec![(PHP_VARS, "php")],
        Lang::Php if matches!(before, Some('>' | ':')) => vec![],
        Lang::Php if before == Some('<') => vec![(HTML_TAGS, "тег")],
        Lang::Php => vec![(PHP_KEYWORDS, "php"), (PHP_FUNCS, "функция")],
        Lang::Html | Lang::Twig if matches!(before, Some('<' | '/')) => vec![(HTML_TAGS, "тег")],
        Lang::Html => vec![(HTML_ATTRS, "атрибут")],
        Lang::Twig => vec![(TWIG_WORDS, "twig"), (HTML_ATTRS, "атрибут")],
        Lang::Js if before == Some('.') => vec![(JS_METHODS, "метод"), (JQUERY, "jQuery")],
        Lang::Js => vec![(JS_KEYWORDS, "js"), (JS_GLOBALS, "js")],
        Lang::Css => vec![(CSS_PROPS, "свойство"), (CSS_VALUES, "значение")],
        Lang::Python => vec![(PYTHON, "python")],
        Lang::Rust => vec![(RUST, "rust")],
        Lang::Other => vec![],
    }
}

fn document_words(doc: &Document, lang: Lang, prefix_lower: &str, out: &mut Vec<String>) {
    let head_line = doc.rope.char_to_line(doc.sel.1);
    let first = head_line.saturating_sub(SCAN_LINES);
    let last = (head_line + SCAN_LINES).min(doc.line_count());
    let mut seen = HashSet::new();
    let mut buf = String::new();
    for line in first..last {
        let slice = doc.rope.line(line);
        if slice.len_bytes() > MAX_SCAN_LINE_BYTES {
            continue;
        }
        buf.clear();
        buf.extend(slice.chunks());
        for word in buf.split(|c: char| !is_word(c, lang)) {
            if word.len() < 3 || word.len() > 80 || word.len() < prefix_lower.len() {
                continue;
            }
            if word.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
                continue;
            }
            if word.to_lowercase().starts_with(prefix_lower) && !seen.contains(word) {
                seen.insert(word.to_owned());
                out.push(word.to_owned());
            }
        }
    }
}

impl Completion {
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
    }

    pub fn refresh(&mut self, doc: &Document, manual: bool) {
        self.close();
        if doc.has_selection() {
            return;
        }
        let lang = lang(&doc.syntax);
        let head = doc.sel.1;
        let mut start = head;
        let mut back = doc.rope.chars_at(head);
        while head - start < 80 && back.prev().is_some_and(|c| is_word(c, lang)) {
            start -= 1;
        }
        let prefix = doc.rope.slice(start..head).to_string();
        if prefix.chars().count() < if manual { 1 } else { 2 } {
            return;
        }
        if prefix.starts_with(|c: char| c.is_ascii_digit()) {
            return;
        }
        let before = (start > 0).then(|| doc.rope.char(start - 1));
        let lower = prefix.to_lowercase();

        let mut words = Vec::new();
        document_words(doc, lang, &lower, &mut words);
        let mut items: Vec<(u8, Item)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut add = |text: &str, kind: &'static str, source: u8| {
            if text == prefix || !seen.insert(text.to_owned()) {
                return;
            }
            let exact_case = text.starts_with(&prefix);
            let rank = source + if exact_case { 0 } else { 2 };
            items.push((
                rank,
                Item {
                    text: text.to_owned(),
                    kind,
                },
            ));
        };
        for (list, kind) in dictionaries(lang, &prefix, before) {
            for word in list.split(' ') {
                if word.len() >= lower.len() && word.to_lowercase().starts_with(&lower) {
                    add(word, kind, 1);
                }
            }
        }
        for word in &words {
            add(word, "текст", 0);
        }
        if items.is_empty() {
            return;
        }
        items.sort_by(|a, b| {
            (a.0, a.1.text.len(), &a.1.text).cmp(&(b.0, b.1.text.len(), &b.1.text))
        });
        items.truncate(MAX_ITEMS);
        self.items = items.into_iter().map(|(_, item)| item).collect();
        self.selected = 0;
        self.start = start;
        self.open = true;
    }

    pub fn step(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        }
    }

    pub fn accept(&mut self, doc: &mut Document) {
        if let Some(item) = self.items.get(self.selected) {
            let head = doc.sel.1;
            if self.start <= head {
                doc.replace(
                    self.start..head,
                    &item.text,
                    crate::document::EditKind::Other,
                );
            }
        }
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(text: &str, syntax: &str, manual: bool) -> (Document, Completion) {
        let mut doc = Document::from_text(text);
        doc.syntax = syntax.into();
        doc.move_to(doc.len(), false);
        let mut c = Completion::default();
        c.refresh(&doc, manual);
        (doc, c)
    }

    fn texts(c: &Completion) -> Vec<&str> {
        c.items.iter().map(|i| i.text.as_str()).collect()
    }

    #[test]
    fn php_functions_and_document_words() {
        let (mut doc, mut c) = complete("<?php\n$html_cache = 1;\nhtml", "PHP", false);
        assert!(c.open);
        let list = texts(&c);
        assert!(list.contains(&"htmlspecialchars"));
        assert!(list.contains(&"html_entity_decode"));
        assert!(!list.contains(&"html"));
        let at = list.iter().position(|t| *t == "htmlspecialchars").unwrap();
        c.selected = at;
        c.accept(&mut doc);
        assert!(doc.rope.to_string().ends_with("\nhtmlspecialchars"));
        assert!(!c.open);
    }

    #[test]
    fn php_variables_come_from_document_and_superglobals() {
        let (_, c) = complete("<?php\n$arResult = [];\n$arParams = [];\n$ar", "PHP", false);
        assert_eq!(texts(&c), vec!["$arParams", "$arResult", "$argc", "$argv"]);
        let (_, c) = complete("<?php\n$_S", "PHP", false);
        assert_eq!(texts(&c), vec!["$_SERVER", "$_SESSION"]);
    }

    #[test]
    fn context_switches_dictionaries() {
        let (_, c) = complete("$('.a').toggleC", "JavaScript (Babel)", false);
        assert_eq!(texts(&c), vec!["toggleClass"]);
        let (_, c) = complete("<?php ?>\n<bl", "PHP", false);
        assert_eq!(texts(&c), vec!["blockquote"]);
        let (_, c) = complete("a { justify-c", "CSS", false);
        assert_eq!(texts(&c), vec!["justify-content"]);
        let (_, c) = complete("{% endb", "HTML (Twig)", false);
        assert_eq!(texts(&c), vec!["endblock"]);
        let (_, c) = complete("let v: Has", "Rust", false);
        assert_eq!(texts(&c), vec!["Hash", "HashMap", "HashSet"]);
    }

    #[test]
    fn short_prefix_needs_manual_trigger_and_no_match_stays_closed() {
        let (_, c) = complete("pr", "Python", false);
        assert_eq!(texts(&c), vec!["print", "property"]);
        let (_, c) = complete("p", "Python", false);
        assert!(!c.open);
        let (_, c) = complete("p", "Python", true);
        assert!(c.open);
        let (_, c) = complete("zzzq", "Python", false);
        assert!(!c.open);
        let (_, c) = complete("12", "Python", true);
        assert!(!c.open);
    }
}
