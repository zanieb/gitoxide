use std::{
    cell::RefCell,
    io::{BufReader, Cursor},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Stdio},
    rc::Rc,
    sync::atomic::AtomicBool,
};

use gix::protocol::transport::{Protocol, client::blocking_io::http};
use gix::remote::{Direction::Fetch, fetch};
use gix_error::ResultExt;
use gix_testtools::TestResult;

/// Run Git's stateless upload-pack service for each HTTP request.
struct GitHttp {
    source: PathBuf,
    requests: Rc<RefCell<Vec<&'static str>>>,
    children: Vec<Child>,
}

impl GitHttp {
    fn upload_pack(&mut self, advertise: bool) -> gix::Result<(ChildStdin, BufReader<ChildStdout>)> {
        let mut cmd = gix_testtools::git_command(&self.source);
        cmd.arg("upload-pack")
            .arg("--stateless-rpc")
            .env("GIT_PROTOCOL", "version=2");
        if advertise {
            cmd.arg("--advertise-refs");
        }
        let mut child = cmd
            .arg(&self.source)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .or_raise(|| gix_error::message("Could not start git upload-pack"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        self.children.push(child);
        Ok((stdin, BufReader::new(stdout)))
    }
}

impl Drop for GitHttp {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.wait();
        }
    }
}

impl http::Http for GitHttp {
    type Headers = Cursor<&'static [u8]>;
    type ResponseBody = BufReader<ChildStdout>;
    type PostBody = ChildStdin;

    fn get(
        &mut self,
        _: &str,
        _: &str,
        _: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> gix::Result<http::GetResponse<Self::Headers, Self::ResponseBody>> {
        self.requests.borrow_mut().push("GET");
        let (_, body) = self.upload_pack(true)?;
        Ok(http::GetResponse {
            headers: Cursor::new(b"Content-Type: application/x-git-upload-pack-advertisement\r\n"),
            body,
        })
    }

    fn post(
        &mut self,
        _: &str,
        _: &str,
        _: impl IntoIterator<Item = impl AsRef<str>>,
        _: http::PostBodyDataKind,
    ) -> gix::Result<http::PostResponse<Self::Headers, Self::ResponseBody, Self::PostBody>> {
        self.requests.borrow_mut().push("POST");
        let (post_body, body) = self.upload_pack(false)?;
        Ok(http::PostResponse {
            headers: Cursor::new(b"Content-Type: application/x-git-upload-pack-result\r\n"),
            body,
            post_body,
        })
    }

    fn configure(&mut self, _: &dyn std::any::Any) -> gix::Result<()> {
        Ok(())
    }
}

#[test]
fn missing_objects_reuse_the_negotiated_transport() -> TestResult {
    if gix_testtools::run_in_isolated_process()? {
        return Ok(());
    }
    let (source, fixture) = super::blocking_and_async_io::try_repo_rw("base")?;
    gix_testtools::git(source.git_dir(), "config uploadpack.allowFilter true")?;
    gix_testtools::git(source.git_dir(), "config uploadpack.allowAnySHA1InWant true")?;
    let mut dest = crate::init_repo_isolated(fixture.path().join("dest"), gix::create::Kind::Bare)?.to_thread_local();
    dest.committer_or_set_generic_fallback()?;
    let requests = Rc::new(RefCell::new(Vec::new()));
    let url = "https://fixture.invalid/repo";
    let transport = http::connect_http(
        GitHttp {
            source: source.git_dir().to_owned(),
            requests: requests.clone(),
            children: Vec::new(),
        },
        gix::url::parse(url)?,
        Protocol::V2,
        false,
    );
    let remote = dest
        .remote_at(url)?
        .with_refspecs(["+HEAD:refs/heads/main"], Fetch)?
        .with_fetch_tags(fetch::Tags::None);
    let stop = AtomicBool::new(false);
    let (_, fetcher) = remote
        .to_connection_with_transport(transport)
        .prepare_fetch(gix::progress::Discard, Default::default())?
        .with_filter(fetch::Filter::BlobNone)
        .receive_with_object_fetcher(gix::progress::Discard, &stop)?;
    assert_eq!(*requests.borrow(), ["GET", "POST", "POST"]);
    let mut fetcher = fetcher.expect("stateless protocol v2 transport");
    fetcher.fetch(&dest, &[], gix::progress::Discard, &stop)?;
    for spec in ["d:file", "g:file"] {
        let id = source.rev_parse_single(spec)?.detach();
        assert!(!dest.has_object(id));
        fetcher.fetch(&dest, &[id], gix::progress::Discard, &stop)?;
        assert_eq!(dest.find_blob(id)?.data, source.find_blob(id)?.data);
    }
    assert_eq!(*requests.borrow(), ["GET", "POST", "POST", "POST", "POST"]);
    let absent = gix::ObjectId::from_hex(b"1010101010101010101010101010101010101010")?;
    assert!(fetcher.fetch(&dest, &[absent], gix::progress::Discard, &stop).is_err());
    assert!(!dest.has_object(absent));
    assert_eq!(requests.borrow().len(), 6);
    for file in std::fs::read_dir(dest.git_dir().join("objects/pack"))? {
        assert_ne!(file?.path().extension(), Some(std::ffi::OsStr::new("keep")));
    }
    Ok(())
}
