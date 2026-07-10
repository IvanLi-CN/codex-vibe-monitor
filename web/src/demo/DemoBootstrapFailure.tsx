export function DemoBootstrapFailure() {
  return (
    <main className="flex min-h-screen items-center justify-center bg-base-200 p-6 text-base-content">
      <section className="w-full max-w-md rounded-lg border border-error/35 bg-base-100 p-6">
        <p className="text-sm font-semibold text-error">Demo 启动失败</p>
        <h1 className="mt-2 text-xl font-semibold">模拟运行环境没有准备完成</h1>
        <p className="mt-3 text-sm leading-6 text-base-content/70">
          此页面不会回退到真实服务。请刷新后重试，或联系站点维护者检查 demo 静态资源。
        </p>
      </section>
    </main>
  )
}
