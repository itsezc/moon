use moon_cache::get_cache_mode;
use moon_emitter::{Event, EventFlow, Subscriber};
use moon_error::MoonError;
use moon_logger::warn;
use moon_workspace::Workspace;
use moonbase::{upload_artifact, MoonbaseError};
use rustc_hash::FxHashMap;
use tokio::task::JoinHandle;

const LOG_TARGET: &str = "moonbase:remote-cache";

// We don't want errors to bubble up and crash the program,
// so instead, we log the error (as a warning) to the console!
fn handle_error(error: MoonbaseError) {
    warn!(target: LOG_TARGET, "{}", error.to_string());
}

pub struct MoonbaseCacheSubscriber {
    hash_urls: FxHashMap<String, Option<String>>,
    requests: Vec<JoinHandle<()>>,
}

impl MoonbaseCacheSubscriber {
    pub fn new() -> Self {
        MoonbaseCacheSubscriber {
            hash_urls: FxHashMap::default(),
            requests: vec![],
        }
    }
}

#[async_trait::async_trait]
impl Subscriber for MoonbaseCacheSubscriber {
    async fn on_emit<'a>(
        &mut self,
        event: &Event<'a>,
        workspace: &Workspace,
    ) -> Result<EventFlow, MoonError> {
        let Some(moonbase) = &workspace.session else {
            return Ok(EventFlow::Continue);
        };

        match event {
            // Check if archive exists in moonbase (the remote) by querying the artifacts endpoint.
            Event::TargetOutputCacheCheck { hash, .. } => {
                if get_cache_mode().is_readable() {
                    match moonbase.get_artifact(hash).await {
                        Ok(Some((artifact, presigned_url))) => {
                            self.hash_urls.insert(artifact.hash, presigned_url);

                            return Ok(EventFlow::Return("remote-cache".into()));
                        }
                        Ok(None) => {
                            // Not remote cached
                        }
                        Err(error) => {
                            handle_error(error);

                            // Fallthrough and check local cache
                        }
                    }
                }
            }

            // The local cache subscriber uses the `TargetOutputArchiving` event to create
            // the tarball. This runs *after* it's been created so that we can upload it.
            Event::TargetOutputArchived {
                archive_path,
                hash,
                target,
                ..
            } => {
                if get_cache_mode().is_writable() && archive_path.exists() {
                    let auth_token = moonbase.auth_token.to_owned();
                    let hash = (*hash).to_owned();
                    let target = target.id.to_owned();
                    let archive_path = archive_path.to_owned();

                    // Run this in the background so we don't slow down the runner
                    // while waiting for very large archives to upload.
                    self.requests.push(tokio::spawn(async move {
                        if let Err(error) =
                            upload_artifact(auth_token, hash, target, archive_path, None).await
                        {
                            handle_error(error);
                        }
                    }));
                }
            }

            // Attempt to download the artifact from the remote cache to `.moon/outputs/<hash>`.
            // This runs *before* the local cache. So if the download is successful, abort
            // the event flow, otherwise continue and let local cache attempt to hydrate.
            Event::TargetOutputHydrating { hash, .. } => {
                if get_cache_mode().is_readable() && self.hash_urls.contains_key(*hash) {
                    let archive_file = workspace.cache.get_hash_archive_path(hash);
                    let download_url = self.hash_urls.get(*hash).unwrap();

                    if let Err(error) = moonbase
                        .download_artifact(hash, &archive_file, download_url)
                        .await
                    {
                        handle_error(error);
                    }

                    // Fallthrough to local cache to handle the actual hydration
                }
            }

            _ => {}
        }

        // For the last event, we want to ensure that all uploads have been completed!
        if event.is_end() {
            for future in self.requests.drain(0..) {
                let _ = future.await;
            }
        }

        Ok(EventFlow::Continue)
    }
}
