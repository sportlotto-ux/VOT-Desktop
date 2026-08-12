use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;

pub async fn read_capped<R>(mut reader: R, limit: usize) -> Vec<u8>
where
    R: AsyncRead + Unpin,
{
    let mut retained = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0u8; 8192];

    while let Ok(read) = reader.read(&mut chunk).await {
        if read == 0 {
            break;
        }

        let remaining = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&chunk[..read.min(remaining)]);
    }

    retained
}
