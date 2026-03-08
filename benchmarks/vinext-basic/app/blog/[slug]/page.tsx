export const dynamic = 'force-dynamic';

export default async function BlogPost({ params }: { params: Promise<{ slug: string }> }) {
  const { slug } = await params;

  return (
    <div>
      <h1>Blog Post: {slug}</h1>
      <p>Post about {slug}</p>
    </div>
  );
}
