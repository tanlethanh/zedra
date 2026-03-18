import { useState, useEffect, useCallback, type FC } from "react";

interface Post {
  id: number;
  title: string;
  body: string;
}

interface PostCardProps {
  post: Post;
  onDelete: (id: number) => void;
}

const PostCard: FC<PostCardProps> = ({ post, onDelete }) => (
  <article className="card">
    <h2>{post.title}</h2>
    <p>{post.body}</p>
    <button onClick={() => onDelete(post.id)} aria-label="Delete post">
      Delete
    </button>
  </article>
);

function usePosts(limit = 10) {
  const [posts, setPosts] = useState<Post[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);

    fetch(`https://jsonplaceholder.typicode.com/posts?_limit=${limit}`)
      .then((r) => {
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        return r.json() as Promise<Post[]>;
      })
      .then((data) => {
        if (!cancelled) {
          setPosts(data);
          setLoading(false);
        }
      })
      .catch((e: Error) => {
        if (!cancelled) {
          setError(e.message);
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [limit]);

  const deletePost = useCallback((id: number) => {
    setPosts((prev) => prev.filter((p) => p.id !== id));
  }, []);

  return { posts, loading, error, deletePost };
}

export default function App() {
  const { posts, loading, error, deletePost } = usePosts(5);

  if (loading) return <p className="spinner">Loading…</p>;
  if (error) return <p className="error">Error: {error}</p>;

  return (
    <main>
      <h1>Posts ({posts.length})</h1>
      <div className="grid">
        {posts.map((post) => (
          <PostCard key={post.id} post={post} onDelete={deletePost} />
        ))}
      </div>
    </main>
  );
}
