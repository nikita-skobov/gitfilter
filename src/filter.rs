use super::export_parser;
use export_parser::StructuredExportObject;
use export_parser::StructuredObjectType;
use std::io::Write;
use std::io::stdout;
use std::io;

// temporary function to test out filtering
pub fn filter_with_cb<T: Write>(stream: T, cb: impl FnMut(&StructuredExportObject) -> bool) -> io::Result<()> {
    let mut stream = stream;
    let mut cb = cb;
    export_parser::parse_git_filter_export_via_channel(None, false,
        |obj| {
            if cb(&obj) {
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
    use std::fs::File;

    #[test]
    fn filter_path_works() {
        let writer = stdout();
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
}
