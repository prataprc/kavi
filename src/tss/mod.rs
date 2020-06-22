use tree_sitter as ts;

use std::{borrow::Borrow, convert::TryFrom, fmt, mem, rc::Rc, result};

use crate::{
    buffer::Buffer,
    colors::{ColorScheme, Highlight},
    term::Style,
    Error, Result,
};

/// Ted style sheet for `toml` format.
pub const TOML: &'static str = include_str!("toml.tss");

/// Ted style sheet for `tss` format, tss stands for ted-style-sheet.
pub const TSS: &'static str = include_str!("tss.tss");

macro_rules! wrap_edge {
    ($edge:expr, $varn:ident) => {{
        *$edge = match mem::replace($edge, Default::default()) {
            e @ Edge::Kind(_) => Edge::$varn(Box::new(e.clone())),
            _ => err_at!(Fatal, msg: format!("unexpected wrap_edge"))?,
        };
        Ok(())
    }};
}

extern "C" {
    fn tree_sitter_tss() -> ts::Language;
}

pub struct Token {
    pub kind: String,
    pub depth: usize,
    pub sibling: usize,
    pub a: usize, // charactor position, inclusive
    pub z: usize, // charactor position, exclusive
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        write!(f, "Token<{},{},{}>", self.kind, self.depth, self.sibling)
    }
}

impl Token {
    pub fn from_node(buf: &Buffer, node: &ts::Node, depth: usize, sibling: usize) -> Token {
        let kind = node.kind().to_string();
        let a = buf.byte_to_char(node.start_byte());
        let z = buf.byte_to_char(node.start_byte());
        Token {
            kind,
            depth,
            sibling,
            a,
            z,
        }
    }
}

#[derive(Clone)]
enum Span {
    Pos(usize, usize),
    Text(String),
}

impl Default for Span {
    fn default() -> Span {
        Span::Pos(0, 0)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Span::{Pos, Text};

        match self {
            Pos(a, z) => write!(f, "tssSpan<{},{}>", *a, *z),
            Text(txt) => write!(f, "tssSpan<{}>", txt),
        }
    }
}

impl Span {
    fn from_node(n: &ts::Node) -> Span {
        Span::Pos(n.start_byte(), n.end_byte())
    }
}

impl Span {
    fn pos_to_text(&mut self, tss: &str) -> Result<()> {
        match self {
            Span::Pos(a, z) => {
                *self = Span::Text(tss[*a..*z].to_string());
                Ok(())
            }
            Span::Text(_) => err_at!(Fatal, msg: format!("unexpected span")),
        }
    }

    fn to_position(&self) -> Result<(usize, usize)> {
        match self {
            Span::Pos(a, z) => Ok((*a, *z)),
            Span::Text(_) => err_at!(Fatal, msg: format!("unexpected span")),
        }
    }

    fn as_text(&self) -> Result<&str> {
        match self {
            Span::Pos(_, _) => err_at!(Fatal, msg: format!("unexpected span")),
            Span::Text(txt) => Ok(txt),
        }
    }
}

#[derive(Default, Clone)]
pub struct Automata {
    patterns: Vec<Rc<Node>>,
    open_nodes: Vec<Node>,
}

impl Automata {
    pub fn from_str(tss: &str, scheme: &ColorScheme) -> Result<Automata> {
        let tree = {
            let mut p = ts::Parser::new();
            let language = unsafe { tree_sitter_tss() };
            err_at!(FailParse, p.set_language(language))?;
            match p.parse(tss, None) {
                Some(tree) => Ok(tree),
                None => err_at!(Fatal, msg: format!("invalid ted style sheet")),
            }?
        };

        let root = {
            assert_eq!(tree.root_node().kind(), "s");
            tree.root_node()
        };

        let mut tc = root.walk();
        let mut patterns = vec![];
        for i in 0..root.child_count() {
            let child = root.child(i).unwrap();
            if child.kind() != "hl_rule" {
                continue;
            }

            let style = {
                let ts_node = child.child_by_field_name("style").unwrap();
                Node::compile_style(ts_node, tss, &mut tc, scheme)?
            };
            let n_selectors: Vec<ts::Node> = {
                let xs = child.child_by_field_name("selectors").unwrap();
                xs.children(&mut tc).collect()
            };
            for n_sel in n_selectors.into_iter() {
                let style = style.clone();
                patterns.push(Rc::new(Node::compile_pattern(
                    n_sel,
                    style.clone(),
                    &mut tc,
                )?))
            }
        }

        Ok(Automata {
            patterns,
            open_nodes: Default::default(),
        })
    }
}

impl fmt::Display for Automata {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        for node in self.patterns.iter() {
            write!(f, "{}\n", node)?;
        }
        Ok(())
    }
}

impl Automata {
    pub fn shift_in(&mut self, token: &Token) -> Result<Option<Style>> {
        use Node::{Child, Descendant, End, Pattern, Sibling, Twin};

        let mut style1: Option<Style> = None;
        let mut ops = vec![];
        for (off, open_node) in self.open_nodes.iter().enumerate() {
            let (next, drop) = open_node.is_match(token)?;
            style1 = match next {
                Some(Node::End(style)) => {
                    ops.push((off, None));
                    Some(style1.unwrap_or(style))
                }
                Some(node) => {
                    ops.push((off, Some(node)));
                    style1
                }
                None if drop => {
                    ops.push((off, None));
                    style1
                }
                None => style1,
            }
        }

        for (off, node) in ops.into_iter() {
            match node {
                Some(node) => {
                    let _ = mem::replace(&mut self.open_nodes[off], node);
                }
                None => {
                    self.open_nodes.remove(off);
                }
            }
        }

        let msg = format!("unreachable");
        let mut style2: Option<Style> = None;
        for node in self.patterns.iter() {
            style2 = match node.borrow() {
                Pattern(e, n) if e.is_match(token)? => match n.borrow() {
                    End(style) => Some(style2.unwrap_or(style.clone())),
                    Pattern(_, _) => {
                        let open_node = n.to_open_node(token)?;
                        self.open_nodes.push(open_node);
                        style2
                    }
                    Twin { .. } => err_at!(Fatal, msg: msg)?,
                    Sibling { .. } => err_at!(Fatal, msg: msg)?,
                    Child { .. } => err_at!(Fatal, msg: msg)?,
                    Descendant { .. } => err_at!(Fatal, msg: msg)?,
                },
                Pattern(_, _) => style2,
                Twin { .. } => err_at!(Fatal, msg: msg)?,
                Sibling { .. } => err_at!(Fatal, msg: msg)?,
                Child { .. } => err_at!(Fatal, msg: msg)?,
                Descendant { .. } => err_at!(Fatal, msg: msg)?,
                End(_) => err_at!(Fatal, msg: msg)?,
            }
        }

        if let Some(style) = style1 {
            Ok(Some(style))
        } else {
            Ok(style2)
        }
    }
}

#[derive(Clone)]
enum Edge {
    Kind(Span),
    Twin(Box<Edge>),
    Sibling(Box<Edge>),
    Child(Box<Edge>),
    Descendant(Box<Edge>),
}

impl Default for Edge {
    fn default() -> Edge {
        Edge::Kind(Default::default())
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Edge::{Child, Descendant, Kind, Sibling, Twin};

        match self {
            Kind(_) => write!(f, "e-kind"),
            Twin(edge) => write!(f, "e-twin<{}>", edge),
            Sibling(edge) => write!(f, "e-sibling<{}>", edge),
            Child(edge) => write!(f, "e-child<{}>", edge),
            Descendant(edge) => write!(f, "e-descendant<{}>", edge),
        }
    }
}

impl Edge {
    fn is_match(&self, token: &Token) -> Result<bool> {
        use Edge::{Child, Descendant, Kind, Sibling, Twin};

        match self {
            Kind(k) => Ok(token.kind == k.as_text()?),
            Twin(_) => err_at!(Fatal, msg: format!("unreachable")),
            Sibling(_) => err_at!(Fatal, msg: format!("unreachable")),
            Child(_) => err_at!(Fatal, msg: format!("unreachable")),
            Descendant(_) => err_at!(Fatal, msg: format!("unreachable")),
        }
    }

    fn pos_to_text(&mut self, tss: &str) -> Result<()> {
        use Edge::{Child, Descendant, Kind, Sibling, Twin};

        match self {
            Kind(cnt) => cnt.pos_to_text(tss)?,
            Twin(edge) => edge.as_mut().pos_to_text(tss)?,
            Sibling(edge) => edge.as_mut().pos_to_text(tss)?,
            Child(edge) => edge.as_mut().pos_to_text(tss)?,
            Descendant(edge) => edge.as_mut().pos_to_text(tss)?,
        }
        Ok(())
    }
}

#[derive(Clone)]
enum Node {
    Pattern(Edge, Rc<Node>),
    Twin {
        edge: Edge,
        next: Rc<Node>,
        depth: usize,
        nth_child: usize,
    },
    Sibling {
        edge: Edge,
        next: Rc<Node>,
        depth: usize,
        nth_child: usize,
    },
    Child {
        edge: Edge,
        next: Rc<Node>,
        depth: usize,
        nth_child: usize,
    },
    Descendant {
        edge: Edge,
        next: Rc<Node>,
        depth: usize,
        nth_child: usize,
    },
    End(Style),
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        use Node::{Child, Descendant, End, Pattern, Sibling, Twin};

        match self {
            Pattern(edge, node) => write!(f, "Pattern<{}> -> {}", edge, node),
            Twin {
                edge,
                next,
                depth,
                nth_child,
            } => write!(f, "Twin<{},{},{}> -> {}", edge, depth, nth_child, next),
            Sibling {
                edge,
                next,
                depth,
                nth_child,
            } => write!(
                f,
                "Sibling<{},{},{}> -> {}",
                edge, depth, /**/ nth_child, next
            ),
            Child {
                edge,
                next,
                depth,
                nth_child,
            } => write!(f, "Child<{},{},{}> -> {}", edge, depth, nth_child, next),
            Descendant {
                edge,
                next,
                depth,
                nth_child,
            } => write!(
                f,
                "Descendant<{},{},{}> -> {}",
                edge, depth, nth_child, next
            ),
            End(style) => write!(f, "End<{}>", style),
        }
    }
}

impl Node {
    fn to_open_node(&self, token: &Token) -> Result<Node> {
        use Edge::{Child, Descendant, Kind, Sibling, Twin};

        match self {
            Node::Pattern(edge, next) => match edge {
                Kind(_) => err_at!(Fatal, msg: format!("unreachable")),
                Twin(ne) => Ok(Node::Twin {
                    edge: ne.as_ref().clone(),
                    next: Rc::clone(next),
                    nth_child: token.sibling,
                    depth: token.depth,
                }),
                Sibling(ne) => Ok(Node::Sibling {
                    edge: ne.as_ref().clone(),
                    next: Rc::clone(next),
                    nth_child: token.sibling,
                    depth: token.depth,
                }),
                Child(ne) => Ok(Node::Child {
                    edge: ne.as_ref().clone(),
                    next: Rc::clone(next),
                    nth_child: token.sibling,
                    depth: token.depth,
                }),
                Descendant(ne) => Ok(Node::Descendant {
                    edge: ne.as_ref().clone(),
                    next: Rc::clone(next),
                    nth_child: token.sibling,
                    depth: token.depth,
                }),
            },
            node @ Node::End(_) => Ok(node.clone()),
            Node::Twin { .. } => err_at!(Fatal, msg: format!("unreachable")),
            Node::Sibling { .. } => err_at!(Fatal, msg: format!("unreachable")),
            Node::Child { .. } => err_at!(Fatal, msg: format!("unreachable")),
            Node::Descendant { .. } => err_at!(Fatal, msg: format!("unreachbl")),
        }
    }

    fn is_match(&self, token: &Token) -> Result<(Option<Node>, bool)> {
        use Node::{Child, Descendant, End, Pattern, Sibling, Twin};

        let (ok, drop, next) = match self {
            Twin {
                edge,
                next,
                depth,
                nth_child,
            } => {
                let ok1 = token.depth == *depth;
                let ok2 = token.sibling == nth_child + 1;
                let ok3 = edge.is_match(token)?;
                (ok1 && ok2 && ok3, !(ok1 && ok2), next)
            }
            Sibling {
                edge,
                next,
                depth,
                nth_child,
            } => {
                let ok1 = token.depth == *depth;
                let ok2 = *nth_child < token.sibling;
                let ok3 = edge.is_match(token)?;
                (ok1 && ok2 && ok3, !ok1, next)
            }
            Child {
                edge, next, depth, ..
            } => {
                let ok1 = token.depth == depth + 1;
                let ok3 = edge.is_match(token)?;
                (ok1 && ok3, token.depth > (depth + 1), next)
            }
            Descendant {
                edge, next, depth, ..
            } => {
                let ok1 = *depth < token.depth;
                let ok3 = edge.is_match(token)?;
                (ok1 && ok3, false, next)
            }
            Pattern(_, _) => err_at!(Fatal, msg: format!("unreachable"))?,
            End(_) => err_at!(Fatal, msg: format!("unreachable"))?,
        };

        if ok {
            Ok((Some(next.to_open_node(token)?), drop))
        } else {
            Ok((None, drop))
        }
    }

    fn as_mut_edge(&mut self) -> &mut Edge {
        use Node::{Child, Descendant, End, Pattern, Sibling, Twin};

        match self {
            Pattern(edge, _) => edge,
            Twin { edge, .. } => edge,
            Sibling { edge, .. } => edge,
            Child { edge, .. } => edge,
            Descendant { edge, .. } => edge,
            End(_) => unreachable!(),
        }
    }

    fn pos_to_text(&mut self, tss: &str) -> Result<()> {
        use Node::{Child, Descendant, End, Pattern, Sibling, Twin};

        match self {
            Pattern(edge, _) => edge.pos_to_text(tss),
            Twin { edge, .. } => edge.pos_to_text(tss),
            Sibling { edge, .. } => edge.pos_to_text(tss),
            Child { edge, .. } => edge.pos_to_text(tss),
            Descendant { edge, .. } => edge.pos_to_text(tss),
            End(_) => Ok(()),
        }
    }
}

impl Node {
    fn compile_style<'a>(
        ts_node: ts::Node<'a>,
        tss: &str,
        tc: &mut ts::TreeCursor<'a>,
        scheme: &ColorScheme,
    ) -> Result<Node> {
        let canvas = scheme.to_style(Highlight::Canvas);
        let style = match ts_node.kind() {
            "highlight" => {
                let mut cont = Span::from_node(&ts_node.child(0).unwrap());
                cont.pos_to_text(tss)?;
                match cont {
                    Span::Text(hl) => {
                        let hl: Highlight = TryFrom::try_from(hl.as_str())?;
                        Ok(scheme.to_style(hl))
                    }
                    _ => err_at!(Fatal, msg: format!("unexpected style")),
                }?
            }
            "properties" => {
                let mut style: Style = Default::default();
                for nprop in ts_node.child(1).unwrap().children(tc) {
                    let nprop = nprop.child_by_field_name("property").unwrap();
                    let mut cont = Span::from_node(&nprop.child(2).unwrap());
                    cont.pos_to_text(tss)?;
                    match nprop.kind() {
                        "fg" => {
                            style.fg = match &cont {
                                Span::Text(color) => {
                                    let fg = Style::to_color(color, &canvas)?;
                                    Ok(fg)
                                }
                                _ => err_at!(Fatal, msg: format!("unexpected")),
                            }?;
                        }
                        "bg" => {
                            style.bg = match &cont {
                                Span::Text(color) => {
                                    let bg = Style::to_color(color, &canvas)?;
                                    Ok(bg)
                                }
                                _ => err_at!(Fatal, msg: format!("unexpected")),
                            }?;
                        }
                        "attrb" | "attribute" => {
                            style.attrs = match &cont {
                                Span::Text(attrs) => Ok(Style::to_attrs(attrs)?),
                                _ => err_at!(Fatal, msg: format!("unexpected")),
                            }?;
                        }
                        _ => err_at!(Fatal, msg: format!("unexpected"))?,
                    }
                }
                style
            }
            kind => err_at!(Fatal, msg: format!("unexpected {:?}", kind))?,
        };

        Ok(Node::End(style))
    }

    fn compile_pattern<'a>(
        ts_node: ts::Node<'a>,
        mut next: Node,
        tc: &mut ts::TreeCursor<'a>,
    ) -> Result<Node> {
        match ts_node.child_count() {
            0 => err_at!(Fatal, msg: format!("unexpected node")),
            1 => Self::compile_sel(ts_node.child(0).unwrap(), next, tc),
            _ => {
                let mut cs: Vec<ts::Node> = ts_node.children(tc).collect();
                cs.reverse();
                let mut iter = cs.into_iter();
                next = Self::compile_sel(iter.next().unwrap(), next, tc)?;
                for child in iter {
                    wrap_edge!(next.as_mut_edge(), Descendant)?;
                    next = Self::compile_sel(child, next, tc)?;
                }
                Ok(next)
            }
        }
    }

    fn compile_sel<'a>(
        ts_node: ts::Node<'a>,
        mut next: Node,
        tc: &mut ts::TreeCursor<'a>,
    ) -> Result<Node> {
        let cs: Vec<ts::Node> = ts_node.children(tc).collect();

        let chd = &cs[0];
        match chd.kind() {
            "sel_kind" => {
                let edge = Edge::Kind(Span::from_node(&chd));
                Ok(Node::Pattern(edge, Rc::new(next)))
            }
            "sel_twins" => {
                next = Self::compile_sel(chd.child(2).unwrap(), next, tc)?;
                wrap_edge!(next.as_mut_edge(), Twin)?;
                Self::compile_sel(chd.child(0).unwrap(), next, tc)
            }
            "sel_siblings" => {
                next = Self::compile_sel(chd.child(2).unwrap(), next, tc)?;
                wrap_edge!(next.as_mut_edge(), Sibling)?;
                Self::compile_sel(chd.child(0).unwrap(), next, tc)
            }
            "sel_child" => {
                next = Self::compile_sel(chd.child(2).unwrap(), next, tc)?;
                wrap_edge!(next.as_mut_edge(), Child)?;
                Self::compile_sel(chd.child(0).unwrap(), next, tc)
            }
            kind => err_at!(Fatal, msg: format!("unexpected {}", kind)),
        }
    }
}

#[cfg(test)]
#[path = "tss_test.rs"]
mod tss_test;