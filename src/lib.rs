use std::io::{BufReader, Error, ErrorKind, BufRead, Read};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use num_cpus;
use std::collections::BinaryHeap;
use std::cmp::Reverse;
// use std::time::Instant;
// use std::time::Duration;

use exechelper;

pub mod export_parser;
use export_parser::StructuredExportObject;

pub enum ParseState {
    BeforeData,
    Data(usize),
    AfterData,
}

pub struct WaitObj {
    pub index: usize,
    pub obj: StructuredExportObject,
}

impl PartialEq for WaitObj {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl Eq for WaitObj {}

impl Ord for WaitObj {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index.cmp(&other.index)
    }
}

impl PartialOrd for WaitObj {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.index.cmp(&other.index))
    }
}

pub struct UnparsedFastExportObject {
    pub before_data_str: String,
    pub data: Vec<u8>,
    pub after_data_str: String,
}

pub type StrOption<'a> = Option<&'a str>;

pub fn make_stdio_err(message: &str) -> Error {
    let kind = ErrorKind::InvalidInput;
    Error::new(kind, message)
}

pub fn make_expected_progress_string(progress_num: u32) -> String {
    let mut s = String::with_capacity(32);
    s.push_str("progress ");
    s.push_str(&progress_num.to_string());
    s.push_str(" objects");
    s
}

/// This 'parser' will only parse the data section
/// and put the rest of the info into a 'metadata' string
/// for future parsing. the rationale is that we need to parse the data section
/// seperately anyway since we need to know when to resume parsing the other
/// sections.
pub fn parse_git_filter_export_with_callback(
    export_branch: Option<String>,
    with_blobs: bool,
    cb: impl FnMut(UnparsedFastExportObject)
) -> Result<(), Error>{
    // let now = Instant::now();
    let export_branch = export_branch.unwrap_or("master".into());
    let mut fast_export_command = vec!["git", "fast-export", "--show-original-ids",
        "--signed-tags=strip", "--tag-of-filtered-object=drop",
        "--fake-missing-tagger","--reference-excluded-parents",
        "--reencode=yes", "--use-done-feature", &export_branch,
        "--progress", "1"
    ];
    if !with_blobs {
        fast_export_command.push("--no-data");
    }

    let mut child = exechelper::spawn_with_env_ex(
        &fast_export_command, &[], &[],
        Some(Stdio::null()), Some(Stdio::null()), Some(Stdio::piped()),
    )?;

    let child_stdout = match child.stdout.take() {
        Some(s) => s,
        None => return Err(make_stdio_err("failed to take child.stdout")),
    };

    let mut cb = cb;
    let mut parse_state = ParseState::BeforeData;
    let mut expected_object = 1;
    let mut expected_progress_string = make_expected_progress_string(expected_object);
    let mut bufreader = BufReader::new(child_stdout);
    // let mut bufreader = BufReader::new(child_stdout).lines();
    
    let mut before_data_str = String::new();
    let mut data_vec: Vec<u8> = vec![];
    let mut after_data_str = String::new();

    loop {
        match parse_state {
            ParseState::BeforeData => {
                let mut line_vec = vec![];
                let num_read = bufreader.read_until('\n' as u8, &mut line_vec)?;
                if num_read == 0 { break; }
                line_vec.pop(); // remove trailing slash
                // at this state, we should be guaranteed that this is valid text data
                // caveat: one of the lines we will parse here would be like:
                // commiter <username> <<useremail@email.email>> <timestamp>
                // could it be possible an 'attacker' could put in a non valid string as
                // the username or email?
                let line = unsafe { String::from_utf8_unchecked(line_vec) };
                if line.starts_with("data ") {
                    let data_size_index = 5; // data + space is 5 chars
                    let data_size = line.get(data_size_index..).unwrap();
                    let data_size: usize = data_size.parse().unwrap();
                    parse_state = ParseState::Data(data_size);
                }
                before_data_str.push_str(&line);
                before_data_str.push('\n');
            }
            ParseState::Data(data_size) => {
                // here we just read the exact number of bytes into a byte vec.
                // this data can potentially be binary data, so we dont convert it to
                // a string. instead, the actual object parser will decide what to do here.
                let mut temp_vec = vec![0; data_size];
                bufreader.read_exact(&mut temp_vec)?;
                parse_state = ParseState::AfterData;
                data_vec = temp_vec;
            }
            ParseState::AfterData => {
                let mut line_vec = vec![];
                let num_read = bufreader.read_until('\n' as u8, &mut line_vec)?;
                if num_read == 0 { break; }
                line_vec.pop(); // remove trailing slash
                let line = unsafe { String::from_utf8_unchecked(line_vec) };
                if line.starts_with(&expected_progress_string) {
                    expected_object += 1;
                    expected_progress_string = make_expected_progress_string(expected_object);

                    let unparsed_obj = UnparsedFastExportObject {
                        before_data_str, data: data_vec, after_data_str
                    };
                    cb(unparsed_obj);

                    before_data_str = String::new();
                    data_vec = vec![];
                    after_data_str = String::new();
                    parse_state = ParseState::BeforeData;
                } else {
                    after_data_str.push_str(&line);
                    after_data_str.push('\n');
                }
            }
        }
    }

    // eprintln!("Spent {:?} on reading the git stream", now.elapsed());
    Ok(())
}

pub fn parse_git_filter_export(
    export_branch: Option<String>,
    with_blobs: bool,
) -> Result<Vec<UnparsedFastExportObject>, Error> {
    let mut unparsed_obj_vec = vec![];
    parse_git_filter_export_with_callback(export_branch, with_blobs, |info| {
        unparsed_obj_vec.push(info);
    })?;
    Ok(unparsed_obj_vec)
}

pub fn parse_git_filter_export_via_channel_and_n_parsing_threads(
    export_branch: Option<String>,
    with_blobs: bool,
    n_parsing_threads: usize,
    cb: impl FnMut(usize),
) {
    let mut cb = cb;
    let mut spawned_threads = vec![];
    let (tx, rx) = mpsc::channel();
    for _ in 0..n_parsing_threads {
        let (parse_tx, parse_rx) = mpsc::channel();
        let parse_consumer_tx_clone = tx.clone();
        let parse_thread = thread::spawn(move || {
            for (counter, received) in parse_rx {
                let parsed = export_parser::parse_into_structured_object(received);
                parse_consumer_tx_clone.send((counter, parsed)).unwrap();
            }
        });
        spawned_threads.push((parse_tx, parse_thread));
    }

    // this transmitter is not doing anything, only the cloned
    // versions of it are in use, so we HAVE to drop it here
    // otherwise our program will hang.
    drop(tx);

    // on the thread that is running the git fast-export,
    // it will alternate passing these UNPARSED messages to one of our
    // parsing threads. the parsing threads (created above)
    // will then pass the PARSED message back to our main thread
    let thread_handle = thread::spawn(move || {
        let mut counter = 0;
        let _ = parse_git_filter_export_with_callback(export_branch, with_blobs, |x| {
            let thread_index = counter % n_parsing_threads as usize;
            let (parse_tx, _) = &spawned_threads[thread_index];
            parse_tx.send((counter, x)).unwrap();
            counter += 1;
        });
    });

    eprintln!("Using threads {}", n_parsing_threads);


    // we want our vec of parsed objects
    // to be in the same order as they were received. so
    // we check the index of the object, and ensure that we are only
    // adding to the out_vec if the entry is consecutive.
    // otherwise we put it into a temporary reverse binary heap
    // which we then keep checking to remove elements from the heap
    // and put them into the out_vec in the correct order
    let mut first_received = false;
    let mut expected = 0;
    let mut out_vec = vec![];
    let mut wait_heap = BinaryHeap::new();
    for received in rx {
        if received.0 == expected {
            out_vec.push(received.1);
            cb(received.0);
            expected += 1;
        } else {
            let wait_obj = WaitObj {
                index: received.0,
                obj: received.1,
            };
            wait_heap.push(Reverse(wait_obj));
        }

        while let Some(wait_obj) = wait_heap.pop() {
            let wait_obj = wait_obj.0;
            if wait_obj.index == expected {
                out_vec.push(wait_obj.obj);
                cb(wait_obj.index);
                expected += 1;
            } else {
                wait_heap.push(Reverse(wait_obj));
                break;
            }
        }

        if !first_received {
            first_received = true;
            eprintln!("Received first PARSED thing at {:?}", std::time::Instant::now());
        }
    }

    let _ = thread_handle.join().unwrap();
    eprintln!("Last received at {:?}", std::time::Instant::now());
}


/// uses mpsc channel to parse a bit faster. the rationale
/// is that the thread that spawns the git fast-export command
/// only needs to:
/// 1. read from the stdout of that command
/// 2. parse the data section
/// then it can pass that parsed data to the main thread
/// which can finish the more intensive parsing/transformations
pub fn parse_git_filter_export_via_channel(
    export_branch: Option<String>,
    with_blobs: bool,
) {
    let cpu_count = num_cpus::get() as isize;
    // minus 2 because we are already using 2 threads.
    let spawn_parser_threads = cpu_count - 2;

    if spawn_parser_threads > 1 {
        return parse_git_filter_export_via_channel_and_n_parsing_threads(
            export_branch, with_blobs, spawn_parser_threads as usize, |_| {});
    }

    // otherwise here we will use only 2 threads: on the main
    // thread we will run the parsing and filtering, and on the spawned
    // thread we will be collecting and splitting the git fast-export output
    let (tx, rx) = mpsc::channel();
    let thread_handle = thread::spawn(move || {
        parse_git_filter_export_with_callback(export_branch, with_blobs, |x| {
            tx.send(x).unwrap();
        })
    });

    let mut parsed_objects = vec![];

    let mut _counter = 0;
    for received in rx {
        parsed_objects.push(export_parser::parse_into_structured_object(received));
        _counter += 1;
    }

    eprintln!("Counted {} objects from git fast-export", parsed_objects.len());
    let _ = thread_handle.join().unwrap();
}

#[cfg(test)]
mod tests {
    // use super::*;
    // use std::io::prelude::*;

    // TODO: whats a unit test? ;)

    #[test]
    fn using_multiple_parsing_threads_keeps_order_the_same() {
        let mut expected_count = 0;
        super::parse_git_filter_export_via_channel_and_n_parsing_threads(
            None, false, 4, |counter| {
                assert_eq!(counter, expected_count);
                expected_count += 1;
            });
    }

    #[test]
    fn test1() {
        let now = std::time::Instant::now();
        super::parse_git_filter_export_via_channel(None, false);
        eprintln!("total time {:?}", now.elapsed());
    }
}
