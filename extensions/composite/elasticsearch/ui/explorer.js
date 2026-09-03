import { Buffer } from "buffer";
import { View, div } from "gpui";
import { h_flex, v_flex } from "gpui-base";
import { Button, Input, InputState } from "gpui-component";
import { close as closeBlob, read as readBlob } from "navop.blob";
import { current } from "navop.context";
import { cancel, close as closeJob, result as jobResult, start, status } from "navop.job";
import { invoke } from "navop.resource";

export default class ElasticsearchExplorer extends View {
  init(_props, cx) {
    this.context = current();
    this.query = InputState.new({ value: "*" });
    this.resource = null;
    this.job = null;
    this.page = "overview";
    this.status = "Connecting";
    this.cluster = null;
    this.indices = [];
    this.selectedIndex = null;
    this.content = null;
    cx.spawn(async (cx) => this.connect(cx));
  }

  render() {
    return h_flex()
      .size_full()
      .min_w_0()
      .min_h_0()
      .child(this.renderSidebar())
      .child(v_flex().flex_1().size_full().min_w_0().p(16).gap(12)
        .child(div().text_size(18).font_semibold().child(this.title()))
        .child(`Status: ${this.status}`)
        .child(this.renderContent()));
  }

  renderSidebar() {
    return v_flex()
      .w(250)
      .h_full()
      .flex_shrink_0()
      .border_r_1()
      .p(12)
      .gap(8)
      .child(div().font_semibold().child(this.context.connection?.name || "Elasticsearch"))
      .child(this.navButton("es-overview", "Cluster overview", "overview"))
      .child(this.navButton("es-indices", "Indices", "indices"))
      .child(this.navButton("es-search-page", "Search", "search"))
      .children(this.indices.map((index) =>
        this.navButton(`es-index-${index.name}`, `  ${index.name}`, `index:${index.name}`),
      ));
  }

  navButton(id, label, page) {
    return new Button(id).ghost().label(label).on_click((_event, cx) => {
      cx.spawn(async (cx) => this.openPage(page, cx));
    });
  }

  renderContent() {
    if (this.page === "search") {
      return v_flex().flex_1().min_h_0().gap(8)
        .child(Input.new(this.query).placeholder("Search text or *"))
        .child(h_flex().gap(8)
          .child(this.actionButton("es-search", "Search", (cx) => this.search(cx)))
          .child(this.actionButton("es-cancel", "Cancel", (cx) => this.cancelSearch(cx))))
        .child(this.resultView());
    }
    return this.resultView();
  }

  resultView() {
    return div().flex_1().min_h_0().overflow_y_scrollbar().whitespace_pre_wrap().child(
      this.content ? JSON.stringify(this.content, null, 2) : "No data",
    );
  }

  actionButton(id, label, run) {
    return new Button(id).label(label).on_click((_event, cx) => {
      cx.spawn(async (cx) => run(cx));
    });
  }

  title() {
    if (this.page.startsWith("index:")) return this.page.slice(6);
    if (this.page === "indices") return "Indices";
    if (this.page === "search") return "Search";
    return "Cluster overview";
  }

  async connect(cx) {
    try {
      const connection = this.context.connection;
      if (!connection?.resource?.handle) throw new Error("Connection resource is missing");
      this.resource = connection.resource.handle;
      this.cluster = await this.resolve(await invoke(this.resource, "elasticsearch/cluster/info", {}));
      await this.loadIndices();
      this.content = this.cluster;
      this.status = "Connected";
    } catch (error) {
      this.status = `Failed: ${error.message}`;
    }
    cx.notify();
  }

  async loadIndices() {
    const result = await this.resolve(await invoke(this.resource, "elasticsearch/index/list", {}));
    this.indices = result.indices || [];
  }

  async openPage(page, cx) {
    this.page = page;
    try {
      if (page === "overview") this.content = this.cluster;
      if (page === "indices") this.content = { indices: this.indices };
      if (page.startsWith("index:")) {
        const name = page.slice(6);
        this.content = await this.resolve(await invoke(
          this.resource,
          "elasticsearch/index/get",
          { name },
        ));
      }
    } catch (error) {
      this.status = `Failed: ${error.message}`;
    }
    cx.notify();
  }

  async search(cx) {
    if (!this.resource) return;
    try {
      if (this.job) await closeJob(this.job);
      const started = await start(this.resource, "elasticsearch/search/async", {
        query: this.query.value().trim(),
      });
      this.job = started.handle;
      while (true) {
        const snapshot = await status(this.job);
        if (snapshot.state === "succeeded") break;
        if (snapshot.state !== "running" && snapshot.state !== "queued") {
          throw new Error(snapshot.message || `Search ${snapshot.state}`);
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      this.content = await this.resolve(await jobResult(this.job));
      await closeJob(this.job);
      this.job = null;
      this.status = "Connected";
    } catch (error) {
      this.status = `Failed: ${error.message}`;
    }
    cx.notify();
  }

  async cancelSearch(cx) {
    if (!this.job) return;
    await cancel(this.job);
    await closeJob(this.job);
    this.job = null;
    this.status = "Search cancelled";
    cx.notify();
  }

  async resolve(result) {
    if (result.kind === "inline") return result.value;
    if (result.kind !== "blob") return result;
    const chunks = [];
    try {
      while (true) {
        const chunk = await readBlob(result.handle, 1024 * 1024);
        chunks.push(Buffer.from(chunk.data, "base64"));
        if (chunk.done) break;
      }
      return JSON.parse(Buffer.concat(chunks).toString("utf8"));
    } finally {
      await closeBlob(result.handle);
    }
  }
}
