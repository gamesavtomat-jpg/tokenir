use futures::StreamExt;
use tokenir_ui::Token;
use tokio_tungstenite::connect_async;

pub struct Client {
    url: String,
}

impl Client {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub async fn subscribe<F, Fut>(&self, mut func: F) -> Result<(), std::io::Error>
    where
        F: FnMut(Token) -> Fut,
        Fut: Future<Output = ()>,
    {
        loop {
            let ws_stream = match connect_async(&self.url).await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            let (_, mut read) = ws_stream.split();

            while let Some(msg) = read.next().await {
                let msg = match msg {
                    Ok(msg) => msg,
                    Err(err) => {
                        eprintln!("{}", err);
                        continue;
                    }
                };

                let token: Token = match serde_json::from_str(&msg.to_string()) {
                    Ok(token) => token,
                    Err(err) => {
                        eprintln!("{}", err);
                        continue;
                    }
                };

                func(token).await;
            }
        }
    }
}
