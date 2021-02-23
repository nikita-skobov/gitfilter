use super::export_parser;
use export_parser::StructuredExportObject;
use std::io::Write;
use std::io;

// temporary function to test out filtering
pub fn filter_with_cb<T: Write>(stream: T, cb: impl FnMut(&mut StructuredExportObject) -> bool) -> io::Result<()> {
    let mut stream = stream;
    let mut cb = cb;
    export_parser::parse_git_filter_export_via_channel(None, false,
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
