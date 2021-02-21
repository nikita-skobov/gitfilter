use super::UnparsedFastExportObject;
use regex::Regex;
use regex::Captures;
use once_cell::sync::OnceCell;
use std::str::SplitWhitespace;

pub trait AsStaticStr { fn as_str() -> &'static str; }

pub struct ReAuthorLine {}

impl AsStaticStr for ReAuthorLine {
    fn as_str() -> &'static str { r"^(?:author|committer) (.*?) ?<(.*?)> (.*?)$" }
}

pub fn get_regex_captures<T: AsStaticStr>(text: &str) -> Option<Captures> {
    static RE: OnceCell<Regex> = OnceCell::new();
    let re = RE.get_or_init(|| {
        Regex::new(T::as_str()).unwrap()
    });

    re.captures(text)
}

pub struct StructuredExportObject {
    pub numthings: u32,
}

pub enum BeforeDataParserMode {
    Initial,
    Reset,
    Commit,
}
use BeforeDataParserMode::*;

pub enum NextWordType {
    Oid,
    Mark,
    CommitRef,
    ResetFrom,
    ResetLine,
    Data,
}
use NextWordType::*;

/// here we diverge from git-fast-import spec a bit.
/// the fast-import spec has several commands, but we only handle
/// two of them: commit and blob.
/// we dont handle tags because we ignore tags. resets are part
/// of the before_data_object so we dont treat it as a seperate object,
/// same goes for feature done. we ignore progress, checkpoint and alias, and the rest
#[derive(Debug)]
pub enum ObjectType<'a> {
    Commit(CommitObject<'a>),
    Blob,
}

impl<'a> Default for ObjectType<'a> {
    fn default() -> Self {
        ObjectType::Commit(CommitObject::default())
    }
}

#[derive(Default, Debug)]
pub struct CommitPerson<'a> {
    pub name: Option<&'a str>,
    pub email: &'a str,
    pub timestr: &'a str,
}

#[derive(Default, Debug)]
pub struct CommitObject<'a> {
    refname: &'a str,
    mark: Option<&'a str>,
    // technically this is optional, but the way we call git-fast-export
    // we should always be given an oid
    oid: &'a str,

    author: CommitPerson<'a>,
    committer: CommitPerson<'a>,
}

#[derive(Default, Debug)]
pub struct BeforeDataObject<'a> {
    has_reset: Option<&'a str>,
    has_reset_from: Option<&'a str>,

    // there are other features but we dont implement them,
    // if we see the keyword 'feature', we assume its "feature done"
    has_feature_done: bool,

    object: ObjectType<'a>,

    data: &'a str,
}

pub fn set_object_property<'a>(
    value: &'a str,
    object: &mut BeforeDataObject<'a>,
    next_word_type: NextWordType,
) {
    if let ObjectType::Commit(commit_obj) = &mut object.object {
        if let Oid = next_word_type {
            commit_obj.oid = value;
        } else if let Mark = next_word_type {
            commit_obj.mark = Some(value);
        }
    } else if let ObjectType::Blob = &mut object.object {
        todo!("aaaa");
    }
}

// Most parsing just needs to see the next word
// this method handles all parsing that only needs to take a single
// word and put it into some property. the property thats being updated
// depends on the value of next_word_type
pub fn parse_next_word<'a>(
    word_split: &mut SplitWhitespace<'a>,
    object: &mut BeforeDataObject<'a>,
    next_word_type: NextWordType,
    parse_mode: &mut BeforeDataParserMode,
) -> Option<()> {
    let next_word = word_split.next()?;
    match next_word_type {
        Oid | Mark => set_object_property(next_word, object, next_word_type),
        CommitRef => {
            let mut commit_obj = CommitObject::default();
            commit_obj.refname = next_word;
            object.object = ObjectType::Commit(commit_obj);
            *parse_mode = BeforeDataParserMode::Commit;
        },
        ResetFrom => {
            object.has_reset_from = Some(next_word);
            *parse_mode = BeforeDataParserMode::Initial;
        },
        ResetLine => {
            // might need to get rid of this check?
            // Im not sure if its possible to see multiple reset
            // lines in a row. If it is, then our parser cannot handle that.
            // if we need to handle this, then wed modify the BeforeDataObject
            // to have a Vec<ResetInfo>
            if object.has_reset.is_some() {
                panic!("This object already has a reset?");
            }
            object.has_reset = Some(next_word);
            *parse_mode = BeforeDataParserMode::Reset;
        },
        Data => {
            object.data = next_word;
        }
    }
    Some(())
}

pub fn parse_author_or_committer_line<'a>(
    line: &'a str,
    object: &mut BeforeDataObject<'a>,
    is_author: bool,
) -> Option<()> {
    let captures = get_regex_captures::<ReAuthorLine>(line)?;
    let name = captures.get(1)?.as_str();
    let email = captures.get(2)?.as_str();
    let timestr = captures.get(3)?.as_str();

    let person = CommitPerson {
        name: if name.is_empty() { None } else { Some(name) },
        email,
        timestr,
    };
    if let ObjectType::Commit(commit_obj) = &mut object.object {
        if is_author {
            commit_obj.author = person;
        } else {
            commit_obj.committer = person;
        }
    }

    Some(())
}

pub fn parse_before_data_line<'a>(
    line: &'a str,
    parse_mode: &mut BeforeDataParserMode,
    object: &mut BeforeDataObject<'a>,
) -> Option<()> {
    // we loop because its convenient to reuse resources and to
    // switch parsing modes
    let mut word_split = line.split_whitespace();
    let first_word = word_split.next()?;

    match parse_mode {
        // in the initial state we are looking for one of several words
        // feature, reset, commit, or blob
        Initial => match first_word {
            "feature" => object.has_feature_done = true,
            "reset" => parse_next_word(&mut word_split, object, ResetLine, parse_mode)?,
            "commit" => parse_next_word(&mut word_split, object, CommitRef, parse_mode)?,
            _ => panic!("Unknown initial parsing?\n{}", line),
        },

        // if we are not in initial parsing mode, then we are parsing
        // reset info, commit info, or blob info.

        // reset is a boring parse because 9999% of the time there is no from <commit-ish>
        // so usually this will just rever back to initial parse mode. but if we do have
        // a from, we check for it here.
        Reset => match first_word {
            "from" => parse_next_word(&mut word_split, object, ResetFrom, parse_mode)?,
            "commit" => parse_next_word(&mut word_split, object, CommitRef, parse_mode)?,
            _ => panic!("Unknown reset parsing?\n{}", line),
        },

        // commit has a lot of stuff to parse out
        Commit => match first_word {
            "mark" => parse_next_word(&mut word_split, object, Mark, parse_mode)?,
            "original-oid" => parse_next_word(&mut word_split, object, Oid, parse_mode)?,
            "author" => parse_author_or_committer_line(line, object, true)?,
            "committer" => parse_author_or_committer_line(line, object, false)?,
            // I dont think we need to handle this because we do --reencode=yes
            "encoding" => (),
            "data" => parse_next_word(&mut word_split, object, Data, parse_mode)?,
            _ => panic!("Unknown commit parsing?\n{}", line),
        },

        // TODO: handle blobs
    }

    Some(())
}

pub fn parse_before_data<'a>(before_data_str: &'a String) -> BeforeDataObject<'a> {
    let mut parser_mode = BeforeDataParserMode::Initial;
    let mut output_obj = BeforeDataObject::default();
    for line in before_data_str.lines() {
        parse_before_data_line(line, &mut parser_mode, &mut output_obj);
    }

    output_obj
}

pub fn parse_into_structured_object(unparsed: UnparsedFastExportObject) -> StructuredExportObject {
    let before_data_obj = parse_before_data(&unparsed.before_data_str);
    // println!("{:#?}", before_data_obj);
    // println!("==========================");
    if let ObjectType::Commit(_commit_obj) = before_data_obj.object {
        // println!("{}", commit_obj.mark.unwrap());
    }

    // TODO: clone the needed properties from before_data_obj
    // into the structured export object
    let numthings = 0;
    StructuredExportObject { numthings }
}


#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn before_data_object_works1() {
        let test_str = r#"
feature done
reset refs/heads/master
commit refs/heads/master
mark :1
original-oid 0c0dffba54e509a82483be2f78bf09451d03babb
author Bryan Bryan <bb@email.com> 1548162866 -0800
committer Bryan Bryan <bb@email.com> 1548162866 -0800
data 12"#;

        let test_string = String::from(test_str);
        let before_obj = parse_before_data(&test_string);
        // println!("{:#?}", before_obj);

        assert_eq!(before_obj.has_feature_done, true);
        assert_eq!(before_obj.has_reset, Some("refs/heads/master"));
        assert_eq!(before_obj.data, "12");
        let obj = if let ObjectType::Commit(c) = before_obj.object {
            c
        } else { panic!("expected commit object") };
        assert_eq!(obj.committer.name, Some("Bryan Bryan"));
        assert_eq!(obj.committer.email, "bb@email.com");
        assert_eq!(obj.author.timestr, "1548162866 -0800");
    }

    #[test]
    fn regex_author_capture_works() {
        let sample1 = "author Bryan Bryan <bb@email.com> 1548162866 -0800";
        let captures = get_regex_captures::<ReAuthorLine>(sample1).unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "Bryan Bryan");
        assert_eq!(captures.get(2).unwrap().as_str(), "bb@email.com");
        assert_eq!(captures.get(3).unwrap().as_str(), "1548162866 -0800");

        // it also works if the starting word is committer
        // and the name can be optional
        let sample2 = "committer <bb@email.com> 1548162866 -0800";
        let captures = get_regex_captures::<ReAuthorLine>(sample2).unwrap();
        assert_eq!(captures.get(1).unwrap().as_str(), "");
        assert_eq!(captures.get(2).unwrap().as_str(), "bb@email.com");
        assert_eq!(captures.get(3).unwrap().as_str(), "1548162866 -0800");
    }
}
