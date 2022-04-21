use std::io::{Error, ErrorKind, Result};

use async_std::channel;
use async_std::io::prelude::BufReadExt;
use async_std::stream::StreamExt;

const LOG_LINE_DELIM: &'static str = "] ";
const REMOTE_ACCESS_PREFIX: &'static str = "[LAN access from remote";

#[derive(Debug, Default)]
struct CommandLineOption<T> {
  parsed: bool,
  value: Option<T>,
}

impl<T> CommandLineOption<T> {
  fn store(self, value: T) -> Self {
    Self {
      parsed: false,
      value: Some(value),
    }
  }
}

#[derive(Debug, Default)]
struct CommandLineOptions {
  input_dir: CommandLineOption<String>,
}

#[derive(Default, Debug)]
struct EmailHead {
  headers: std::collections::HashMap<String, String>,
  done: bool,
}

impl EmailHead {
  fn push<S>(&mut self, item: S) -> bool
  where
    S: std::convert::AsRef<str>,
  {
    if self.done == true {
      return false;
    }

    if item.as_ref().len() == 0 {
      self.done = true;
      return true;
    }

    let mut parts = item.as_ref().split(": ");
    let (key, value) = parts.next().zip(parts.next()).unwrap_or_else(|| ("".into(), "".into()));
    self.headers.insert(key.to_string(), value.to_string());

    true
  }
}

struct RemoteAccess {
  address: String,
}

async fn parse<S>(input: S, output: channel::Sender<RemoteAccess>) -> Result<()>
where
  S: std::convert::AsRef<std::path::Path>,
{
  let mut file = async_std::fs::File::open(input.as_ref()).await?;
  let reader = async_std::io::BufReader::new(&mut file);

  let mut lines = reader.lines();
  let mut head = EmailHead::default();
  let mut peripheral = Vec::with_capacity(100);

  while let Some(Ok(line)) = lines.next().await {
    if head.push(&line) {
      continue;
    }

    match &line.split(LOG_LINE_DELIM).collect::<Vec<&str>>()[..] {
      [REMOTE_ACCESS_PREFIX, value] => match &value.split(" ").collect::<Vec<&str>>()[..] {
        ["from", peer, "to", _mine, _day, _mon, _date, _time] => {
          let mut bits = peer.split(":");
          let (peer_ip, _peer_port) = (bits.next(), bits.next());
          let key = format!("{}", peer_ip.unwrap_or("unknown"));
          output.send(RemoteAccess { address: key }).await.map_err(|error| {
            println!("WARNING - {error}");
            Error::new(ErrorKind::Other, format!("{error}"))
          })?;
        }
        other => println!("unrecognized access log - '{}'", other.join("|")),
      },

      other => peripheral.push(other.join(LOG_LINE_DELIM)),
    }
  }

  Ok(())
}

async fn run(mut options: CommandLineOptions) -> Result<()> {
  let dir = options
    .input_dir
    .value
    .take()
    .and_then(|attempt| {
      let path = std::path::Path::new(&attempt);

      if path.is_dir() {
        Some(std::path::PathBuf::from(path))
      } else {
        None
      }
    })
    .ok_or_else(|| Error::new(ErrorKind::Other, "no '--input-dur'"))?;

  let mut mappings = std::collections::HashMap::with_capacity(1000);

  let (sender, receiver) = channel::bounded(4);
  let mut entries = dir.read_dir()?;

  println!("scanning '{dir:?}'");

  while let Some(Ok(entry)) = entries.next() {
    if entry.path().is_dir() == true {
      continue;
    }

    let name = entry.file_name();
    println!("checking '{name:?}'");

    async_std::task::spawn(parse(entry.path(), sender.clone()));
  }

  // With all tasks spawned, drop our copy of the sender.
  drop(sender);

  while let Ok(next) = receiver.recv().await {
    let existing = mappings.remove(&next.address).unwrap_or(0u32);

    mappings.insert(next.address, existing + 1);
  }

  println!("done receiving");

  let mut hidden = 0;
  let total = mappings.len();

  for (key, value) in mappings.into_iter() {
    if value > 100 {
      println!("{:?}: {:?}", key, value);
    } else {
      hidden += 1;
    }
  }

  println!("{hidden} hidden entries (of {})", total);

  Ok(())
}

fn main() -> Result<()> {
  let opts = std::env::args().fold(CommandLineOptions::default(), |mut opts, item| {
    if opts.input_dir.parsed {
      opts.input_dir = opts.input_dir.store(item.clone());
    }

    if item == "--input-dir" {
      opts.input_dir.parsed = true;
    }

    opts
  });

  async_std::task::block_on(run(opts))
}
