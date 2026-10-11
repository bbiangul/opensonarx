use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::StreamProtocol;
use libp2p::request_response;
use oi_types::proto::opensonarx::{QueryRequest, QueryResponse, ReplicationReq, ReplicationResp};
use prost::Message;
use std::io;

/// Protocol identifier for OpenSonarX queries.
pub const QUERY_PROTOCOL: StreamProtocol = StreamProtocol::new("/opensonarx/query/1.0.0");

/// Protocol identifier for OpenSonarX replication.
pub const REPLICATION_PROTOCOL: StreamProtocol = StreamProtocol::new("/opensonarx/replicate/1.0.0");

/// Codec for encoding/decoding query requests and responses using prost.
#[derive(Debug, Clone, Default)]
pub struct QueryCodec;

/// Codec for encoding/decoding replication requests and responses using prost.
#[derive(Debug, Clone, Default)]
pub struct ReplicationCodec;

/// Read a length-prefixed (4-byte big-endian) prost message from an async reader.
async fn read_length_prefixed<T, M>(io: &mut T) -> io::Result<M>
where
    T: AsyncRead + Unpin + Send,
    M: Message + Default,
{
    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    // Guard against unreasonable message sizes (16 MiB limit)
    if len > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message too large: {len} bytes"),
        ));
    }

    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;

    M::decode(&buf[..]).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write a length-prefixed (4-byte big-endian) prost message to an async writer.
async fn write_length_prefixed<T, M>(io: &mut T, msg: M) -> io::Result<()>
where
    T: AsyncWrite + Unpin + Send,
    M: Message,
{
    let buf = msg.encode_to_vec();
    let len = buf.len() as u32;
    io.write_all(&len.to_be_bytes()).await?;
    io.write_all(&buf).await?;
    io.close().await?;
    Ok(())
}

#[async_trait]
impl request_response::Codec for QueryCodec {
    type Protocol = StreamProtocol;
    type Request = QueryRequest;
    type Response = QueryResponse;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_length_prefixed(io).await
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_length_prefixed(io).await
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, req).await
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, res).await
    }
}

#[async_trait]
impl request_response::Codec for ReplicationCodec {
    type Protocol = StreamProtocol;
    type Request = ReplicationReq;
    type Response = ReplicationResp;

    async fn read_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_length_prefixed(io).await
    }

    async fn read_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_length_prefixed(io).await
    }

    async fn write_request<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, req).await
    }

    async fn write_response<T>(
        &mut self,
        _protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, res).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::io::Cursor;
    use libp2p::request_response::Codec as _;
    use oi_types::proto::opensonarx::{
        DocumentChunk, Filters, ReplicationReq, ReplicationResp, ResponseMeta,
    };

    #[tokio::test]
    async fn test_codec_roundtrip_request() {
        let mut codec = QueryCodec;
        let protocol = QUERY_PROTOCOL.clone();

        let original = QueryRequest {
            request_id: "req-001".to_string(),
            query_embedding: vec![1, 2, 3, 4],
            query_text: "what is opensonarx".to_string(),
            filters: Some(Filters {
                min_stake_usd: 100,
                freshness: "1d".to_string(),
                types: vec!["text".to_string()],
                min_reputation: 1.0,
                shards: vec![],
                entities: vec![],
                content_kinds: vec![],
                locale: String::new(),
                gl: String::new(),
                site: String::new(),
                min_trust: String::new(),
            }),
            limit: 5,
            signed_ticket: vec![],
            sender_pubkey: "SoLpubkey123".to_string(),
            nonce: 42,
            claims: false,
        };

        // Encode
        let mut buf = Vec::new();
        codec
            .write_request(&protocol, &mut buf, original.clone())
            .await
            .expect("write_request failed");

        // Decode
        let mut cursor = Cursor::new(buf);
        let decoded = codec
            .read_request(&protocol, &mut cursor)
            .await
            .expect("read_request failed");

        assert_eq!(decoded.request_id, original.request_id);
        assert_eq!(decoded.query_text, original.query_text);
        assert_eq!(decoded.query_embedding, original.query_embedding);
        assert_eq!(decoded.limit, original.limit);
        assert_eq!(decoded.sender_pubkey, original.sender_pubkey);
        assert_eq!(decoded.nonce, original.nonce);
        assert_eq!(decoded.filters.unwrap().min_stake_usd, 100);
    }

    #[tokio::test]
    async fn test_codec_roundtrip_response() {
        let mut codec = QueryCodec;
        let protocol = QUERY_PROTOCOL.clone();

        let original = QueryResponse {
            request_id: "req-001".to_string(),
            meta: Some(ResponseMeta {
                latency_ms: 42,
                cost_micro_usd: 1500,
                shard_id: "tech-v1".to_string(),
                node_id: "node-abc".to_string(),
                model_version: "minilm-v1".to_string(),
            }),
            results: vec![],
            error: None,
        };

        // Encode
        let mut buf = Vec::new();
        codec
            .write_response(&protocol, &mut buf, original.clone())
            .await
            .expect("write_response failed");

        // Decode
        let mut cursor = Cursor::new(buf);
        let decoded = codec
            .read_response(&protocol, &mut cursor)
            .await
            .expect("read_response failed");

        assert_eq!(decoded.request_id, original.request_id);
        let meta = decoded.meta.unwrap();
        assert_eq!(meta.latency_ms, 42);
        assert_eq!(meta.cost_micro_usd, 1500);
        assert_eq!(meta.shard_id, "tech-v1");
        assert_eq!(meta.node_id, "node-abc");
    }

    #[tokio::test]
    async fn test_replication_codec_roundtrip_request() {
        let mut codec = ReplicationCodec;
        let protocol = REPLICATION_PROTOCOL.clone();

        let original = ReplicationReq {
            shard_id: "topic-technology".to_string(),
            from_seq: 42,
            max_chunks: 100,
        };

        let mut buf = Vec::new();
        codec
            .write_request(&protocol, &mut buf, original.clone())
            .await
            .expect("write_request failed");

        let mut cursor = Cursor::new(buf);
        let decoded = codec
            .read_request(&protocol, &mut cursor)
            .await
            .expect("read_request failed");

        assert_eq!(decoded.shard_id, original.shard_id);
        assert_eq!(decoded.from_seq, original.from_seq);
        assert_eq!(decoded.max_chunks, original.max_chunks);
    }

    #[tokio::test]
    async fn test_replication_codec_roundtrip_response() {
        let mut codec = ReplicationCodec;
        let protocol = REPLICATION_PROTOCOL.clone();

        let original = ReplicationResp {
            chunks: vec![DocumentChunk {
                chunk_id: "c1".to_string(),
                url: "https://example.com/page".to_string(),
                content: "some content".to_string(),
                embedding: vec![0, 0, 128, 63], // f32 LE for 1.0
                metadata: [("title".to_string(), "Test".to_string())].into(),
                content_hash: "abc123".to_string(),
                seq: 1,
            }],
            latest_seq: 1,
        };

        let mut buf = Vec::new();
        codec
            .write_response(&protocol, &mut buf, original.clone())
            .await
            .expect("write_response failed");

        let mut cursor = Cursor::new(buf);
        let decoded = codec
            .read_response(&protocol, &mut cursor)
            .await
            .expect("read_response failed");

        assert_eq!(decoded.chunks.len(), 1);
        assert_eq!(decoded.chunks[0].chunk_id, "c1");
        assert_eq!(decoded.chunks[0].url, "https://example.com/page");
        assert_eq!(decoded.chunks[0].content, "some content");
        assert_eq!(decoded.chunks[0].content_hash, "abc123");
        assert_eq!(decoded.latest_seq, 1);
    }
}
