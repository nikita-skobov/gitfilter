use super::*;
pub mod structured_parse;
pub use structured_parse::*;

pub mod unstructured_parse;
pub use unstructured_parse::*;


use std::io::Error;
use std::sync::mpsc;
use std::thread;
use num_cpus;
use std::collections::BinaryHeap;
use std::cmp::Reverse;
// use std::time::Instant;
// use std::time::Duration;

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
