use super::export_parser;
use export_parser::StructuredExportObject;
use std::io::Write;
use std::io;

pub enum FilterRule {
    FilterRulePathInclude(String),
    FilterRulePathExclude(String),
}

pub type FilterRules = Vec<FilterRule>;

/// Filter options are
/// just the initial options passed to initiate
/// the filtering operation. the actual
/// rules that determine how something is filtered is in
/// `FilterRules`
#[derive(Debug, Default)]
pub struct FilterOptions<T: Write> {
    pub stream: T,
    /// defaults to master
    pub branch: Option<String>,
    pub with_blobs: bool,
    // TODO:
    // pub num_threads: Option<usize>,
}

impl<T: Write> From<T> for FilterOptions<T> {
    fn from(orig: T) -> Self {
        FilterOptions {
            stream: orig,
            branch: None,
            with_blobs: false,
        }
    }
}

pub fn filter_with_rules<T: Write>(
    filter_options: FilterOptions<T>,
    filter_rules: FilterRules,
) -> io::Result<()> {
    let cb = |obj: &mut StructuredExportObject| -> bool {
        true
    };
    filter_with_cb(filter_options, cb)
}

// temporary function to test out filtering
pub fn filter_with_cb<T: Write, F: Into<FilterOptions<T>>>(
    options: F,
    cb: impl FnMut(&mut StructuredExportObject) -> bool
) -> io::Result<()> {
    let options: FilterOptions<T> = options.into();
    let mut stream = options.stream;
    let mut cb = cb;
    export_parser::parse_git_filter_export_via_channel(options.branch, options.with_blobs,
        |mut obj| {
            if cb(&mut obj) {
                return export_parser::write_to_stream(&mut stream, obj);
            }
            Ok(())
        }
    )?;

    stream.write_all(b"done\n")?;

    Ok(())
}


#[cfg(test)]
mod test {
    use super::*;
    use std::io::sink;
    use std::io::Cursor;
    use std::io::Read;
    use export_parser::StructuredObjectType;

    #[test]
    fn filter_path_works() {
        let writer = sink();
        filter_with_cb(writer, |obj| {
            match &obj.object_type {
                StructuredObjectType::Blob(_) => true,
                StructuredObjectType::Commit(commit_obj) => {
                    if commit_obj.committer.email.contains("jerry") {
                        false
                    } else {
                        true
                    }
                }
            }
        }).unwrap();
    }

    #[test]
    fn can_modify_filter_objects() {
        let mut writer = Cursor::new(vec![]);
        filter_with_cb(&mut writer, |obj| {
            if let Some(reset) = &mut obj.has_reset {
                *reset = "refs/heads/NEWBRANCHNAMEAAAAAAA".into();
            }
            match &mut obj.object_type {
                StructuredObjectType::Blob(_) => true,
                StructuredObjectType::Commit(commit_obj) => {
                    commit_obj.commit_ref = commit_obj.commit_ref.replace("master", "NEWBRANCHNAMEAAAAAAA");
                    true
                }
            }
        }).unwrap();

        let mut s = String::from("");
        writer.set_position(0);
        writer.read_to_string(&mut s).unwrap();
        assert!(s.contains("refs/heads/NEWBRANCHNAMEAAAAAAA"));
        assert!(!s.contains("refs/heads/master"));
    }
}
