export default {
  fetch(request: Request): Response {
    if (request.method !== "GET" && request.method !== "HEAD") {
      return new Response("Method not allowed", { status: 405, headers: { Allow: "GET, HEAD" } })
    }
    const target = new URL(request.url)
    target.protocol = "https:"
    target.port = ""
    target.hostname = "download.tanh.moe"
    return Response.redirect(target.toString(), 302)
  },
}
