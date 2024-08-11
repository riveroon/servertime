use std::io;

const URL: &str = "ysweb.yonsei.ac.kr";
const PORT: u16 = 443;
const DEPTH: u32 = 10;



fn main() -> io::Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut client = servertime::Client::new(URL, PORT).await?;

            for _ in 0..DEPTH {
                client.run().await?;
            }

            let range = client.range();
            println!("{range}");
            
            Ok(())
        })
}
