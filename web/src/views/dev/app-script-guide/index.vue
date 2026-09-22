<template>
  <div class="app-script-guide">
    <el-card shadow="never">
      <template #header>
        <div class="card-header">
          <div class="header-left">
            <span class="title">应用脚本编写指南</span>
            <el-tag type="info" size="small" style="margin-left: 8px">开发</el-tag>
          </div>
        </div>
      </template>

      <div class="guide-body">
        <aside class="guide-toc">
          <div class="toc-title">本页目录</div>
          <a
            v-for="t in toc"
            :key="t.id"
            :href="'#' + t.id"
            class="toc-item"
            title="跳转到本页对应章节"
            @click.prevent="scrollToSection(t.id)"
          >{{ t.label }}</a>
        </aside>

        <main class="guide-content">
          <p class="lead">
            为应用商店仓库编写包描述（<code>app.yaml</code>）与生命周期脚本（安装 / 卸载 /
            升级）的完整约定。内容与
            <code>zapexec</code> / <code>zapd</code> 实现保持同步，脚本作者请以此为准。
          </p>

          <!-- 一、包结构 -->
          <h2 id="sec-package">一、包结构与目录约定</h2>
          <p>仓库内每个应用是一个 <code>category/name</code> 目录（分类/名称），包路径仅允许 ASCII 字母、数字、<code>-</code>、<code>_</code>：</p>
          <pre class="code">{{ codes.tree }}</pre>
          <p class="sec-sub"><strong>官方分类（category）</strong>＝仓库一级目录名，当前共五类：</p>
          <pre class="code">{{ codes.categories }}</pre>
          <el-alert type="warning" :closable="false" class="doc-tip">
            商店列表仅扫描这五类目录；放在其它目录名下的包不会被收录，<code>app.yaml</code> 的 <code>category</code> 字段须与此保持一致（缺省取父目录名）。
          </el-alert>
          <ul>
            <li><code>app.yaml</code>：应用描述，见第二节；</li>
            <li><code>install.sh</code> / <code>uninstall.sh</code> / <code>upgrade.sh</code>：默认生命周期脚本文件名，可用 <code>scripts</code> 字段覆盖（见第三节）；</li>
            <li>其余文件（源码、配置模板、编译资源等）随包下发，运行时位于快照目录内，由脚本自行引用。</li>
          </ul>
          <el-alert type="warning" :closable="false" class="doc-tip">
            脚本实际执行的是「本次运行的快照副本」，而不是仓库源目录里的同名文件，详见第四节「执行模型」。
          </el-alert>

          <!-- 二、app.yaml -->
          <h2 id="sec-appyaml">二、app.yaml 字段说明</h2>
          <table class="doc-table">
            <thead>
              <tr><th style="width: 180px">字段</th><th style="width: 120px">类型</th><th>说明</th></tr>
            </thead>
            <tbody>
              <tr><td><code>name</code></td><td>string</td><td>包名，缺省取目录名</td></tr>
              <tr><td><code>title</code></td><td>string</td><td>显示名称</td></tr>
              <tr><td><code>category</code></td><td>string</td><td>分类，官方五类之一（<code>infra</code> / <code>application</code> / <code>webapps</code> / <code>database</code> / <code>library</code>），缺省取父目录名；不在五类目录下的包不会进入商店可用列表</td></tr>
              <tr><td><code>description</code></td><td>string</td><td>简介</td></tr>
              <tr><td><code>version</code></td><td>string / array</td><td>单值或数组（如 <code>[1.24.0, 1.22.1]</code>）；数组表示支持安装的多个版本，首个为默认版本</td></tr>
              <tr><td><code>deps</code></td><td>string[]</td><td>兼容旧写法：依赖名列表</td></tr>
              <tr><td><code>dependencies</code></td><td>map</td><td>依赖库名 → 版本要求（如 <code>openssl: 1.1.1w</code>）</td></tr>
              <tr><td><code>actions</code></td><td>map</td><td>自定义操作按钮：动作键 → 文案（如 <code>build: 编译安装</code>）。发起安装/升级时 env 注入 <code>ACTION=动作键</code>，选项按动作键区分（见第六节）</td></tr>
              <tr><td><code>scripts</code></td><td>map</td><td>脚本文件名覆盖：<code>install / uninstall / upgrade</code> → 文件名</td></tr>
              <tr><td><code>options</code></td><td>map / list</td><td>安装/升级可选项定义：动作键 → 选项列表；顶层直接写列表等价于作用于 install 动作（见第六节）</td></tr>
              <tr><td><code>allow_multiple_instances</code></td><td>bool</td><td>为 true 时已安装仍可再装其它版本（多实例）</td></tr>
              <tr><td><code>roles</code></td><td>string / string[]</td><td>可「浏览 + 安装 / 升级」此包的角色白名单（如 <code>[admin, user]</code>）。<strong>空 / 未声明 = 默认仅 admin 可见可操作</strong>；声明后 admin 恒可操作，命中白名单的角色也能在商店看到并安装 / 升级（Web 商店隐藏未命中角色、install / upgrade 后端二次校验）。典型用途：<code>webapps</code> 类产品（如 wordpress）声明 roles 开放给普通用户角色，由用户自己到商店一键安装</td></tr>
              <tr><td><code>default_port</code></td><td>int</td><td>默认端口（仅展示用途）</td></tr>
              <tr><td><code>version_meta</code></td><td>map</td><td>版本 → 附加元数据。合并入口（多家族一包，如 MySQL / MariaDB）的核心：前端分组展示、zapexec 下发家族、脚本分流均以它为准（字段与用法见第十节「合并入口示例」）</td></tr>
            </tbody>
          </table>

          <!-- 三、脚本与生命周期 -->
          <h2 id="sec-lifecycle">三、生命周期脚本与升级策略</h2>
          <table class="doc-table">
            <thead>
              <tr><th>流程</th><th style="width: 200px">脚本解析</th><th>说明</th></tr>
            </thead>
            <tbody>
              <tr>
                <td>安装</td>
                <td><code>scripts.install</code>，缺省 <code>install.sh</code></td>
                <td>包未安装时执行；成功后系统在 <code>APP_PATH</code> 写入运行元数据 <code>meta.yaml</code>（版本 / 来源 / 安装时间 / run_id），脚本须自行登记实例信息 <code>info.yaml</code>（见下「实例登记」）</td>
              </tr>
              <tr>
                <td>卸载</td>
                <td><code>scripts.uninstall</code>，缺省 <code>uninstall.sh</code></td>
                <td>仅已安装时可执行；成功后系统自动删除 <code>APP_PATH</code>（含 <code>meta.yaml</code> / <code>info.yaml</code>），需要保留的备份请在脚本内自行处理</td>
              </tr>
              <tr>
                <td>升级</td>
                <td>存在 <code>upgrade.sh</code> 则执行；否则自动回退为「先 <code>uninstall.sh</code>、后 <code>install.sh</code>」两段</td>
                <td>旧版本目录清理由脚本自理（<code>APP_OLD_VERSION</code> 携带旧版本号）；两段式策略中卸载阶段请勿删除还需复用的数据。成功后系统刷新 <code>meta.yaml</code>；若安装目录 / 服务名有变，脚本应同步更新 <code>info.yaml</code></td>
              </tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>实例登记（info.yaml）</strong>：<code>APP_PATH</code>（<code>$ZAP_PATH/data/apps/&lt;category&gt;/&lt;name&gt;/</code>）下两类记录分工——<code>meta.yaml</code> 由系统在成功时写入（版本 / 来源 / 安装时间 / run_id 等运行元数据，脚本不要写）；<code>info.yaml</code> 由安装 / 升级脚本在结束前自行登记，Web 端「已安装」据此展示实例、探测状态并支持启停：</p>
          <pre class="code">{{ codes.infoYaml }}</pre>
          <table class="doc-table">
            <thead>
              <tr><th style="width: 140px">字段</th><th style="width: 110px">类型</th><th>说明</th></tr>
            </thead>
            <tbody>
              <tr><td><code>svc_name</code></td><td>string</td><td>守护型应用填 systemd unit 名（如 <code>mysql</code> / <code>nginx</code> / <code>php-fpm-85</code>），状态探测与面板启停走 systemctl</td></tr>
              <tr><td><code>instance</code></td><td>string</td><td>实例展示标识（如 <code>php85</code>、<code>openssl1011</code>）</td></tr>
              <tr><td><code>install_dir</code></td><td>string</td><td>软件本体实际安装目录（位于 <code>$APPS_DIR</code> 下），日志定位 / 「打开目录」用</td></tr>
              <tr><td><code>config_file</code></td><td>string</td><td>主配置文件绝对路径（单文件快捷编辑入口）</td></tr>
              <tr><td><code>config_files</code></td><td>string[] / {path,label}[]</td><td>可编辑文件列表（可选）；「已安装」详情据此提供多个配置文件编辑入口，每项为纯路径或 <code>{path, label}</code>；未填时回退 <code>config_file</code></td></tr>
              <tr><td><code>pid_file</code></td><td>string</td><td>pid 文件路径；守护型填写，作无 systemd 环境下的兜底探活</td></tr>
              <tr><td><code>expose</code></td><td>string / string[]</td><td>暴露入口：<code>tcp:80</code>、<code>unix:/run/xxx.sock</code> 等，可多行数组；无则 <code>none</code></td></tr>
              <tr><td><code>tags</code></td><td>string[]</td><td>分类 / 特性标签（如 <code>infra</code>、<code>library</code>）</td></tr>
            </tbody>
          </table>
          <el-alert type="info" :closable="false" class="doc-tip">
            无守护进程的库类（如 openssl / libpng / libpcre2）不写 <code>svc_name</code> / <code>pid_file</code>，状态由系统返回 unknown；需面板支持「启动 / 停止 / 状态」的守护型应用务必登记 <code>svc_name</code>。
          </el-alert>

          <ul>
            <li><strong>退出码约定</strong>：任一步骤退出码非 0 即视为失败并中断后续步骤，任务最终以最后一次非 0 退出码结束；</li>
            <li><strong>输出</strong>：脚本 <code>stdout / stderr</code> 实时追加进 <code>run-&lt;run_id&gt;.log</code>，Web 端可跟踪，失败排查请把原因打印到输出；</li>
            <li>脚本以 root 运行，但环境是「清空 + 安全白名单 PATH」的纯净环境（见第五节），不要依赖宿主机自定义变量。</li>
          </ul>

          <!-- 四、执行模型 -->
          <h2 id="sec-model">四、执行模型：快照副本 + 失败保留重跑</h2>
          <p>每次 install / uninstall / upgrade 启动前，系统把仓库中该包目录整体复制为本次运行的脚本快照：</p>
          <pre class="code">{{ codes.model }}</pre>
          <ul>
            <li><code>PKG_PATH</code> 指向的是这个快照目录（含 <code>app.yaml</code>、脚本、<code>options.env</code> 等），不是仓库源目录；运行中修改仓库不会影响已在队列/运行中的任务；</li>
            <li><code>run.json</code> 记录本次运行的原始参数（动作 / 版本 / 选项 / 操作者与运行模式），供重跑还原环境（重跑时 <code>ZAP_USER</code> / <code>ZAP_RUN_MODE</code> 与原任务保持一致）；</li>
            <li>全部步骤退出码为 0（成功）后，系统自动清理 <code>runs/&lt;run_id&gt;</code>（脚本快照与 <code>build/</code> 编译目录一并清理），避免磁盘堆积；</li>
            <li>失败（任一退出码非 0）则保留整个运行现场：<code>pkg/</code> 内脚本与 <code>options.env</code> 可在「系统设置 → 任务队列」的任务列表里用「编辑脚本」读取/编辑，<code>build/</code> 编译残留一并保留便于排查，之后「编辑脚本 / 重跑」复用同一快照重试；</li>
            <li>全局串行队列：同一时间仅执行一个脚本任务，后续任务先排队，日志中会提示「任务进入执行队列，等待前序任务完成后自动开始」；</li>
            <li>日志结束时追加一行 <code>__ZAP_DONE__ &lt;退出码&gt;</code> 作为完成标记。</li>
          </ul>

          <!-- 五、环境变量 -->
          <h2 id="sec-env">五、注入的环境变量</h2>
          <p>子进程通过 <code>env_clear()</code> 清空宿主机环境，仅注入白名单 <code>PATH</code> 与下表变量（脚本可放心使用，不会被污染）：</p>
          <table class="doc-table">
            <thead>
              <tr><th style="width: 200px">变量</th><th>含义</th></tr>
            </thead>
            <tbody>
              <tr><td><code>PATH</code></td><td>安全白名单：/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin（不可覆盖）</td></tr>
              <tr><td><code>ZAP_PATH</code></td><td>面板安装根目录，缺省 /usr/local/zap</td></tr>
              <tr><td><code>ZAPCTL</code></td><td>zapctl 可执行文件路径（<code>$ZAP_PATH/zapctl</code>）</td></tr>
              <tr><td><code>APPS_DIR</code></td><td>软件本体安装根目录（默认 <code>/usr/local/apps</code>；zapd / zapexec 启动时可经 <code>ZAP_APPS_DIR</code> 覆盖）。<code>configure --prefix</code> 等最终安装目标以此作基准拼接（如 <code>$APPS_DIR/nginx-1.24.0</code>），不要硬编码系统目录</td></tr>
              <tr><td><code>LOG_FILE</code></td><td>本次运行日志绝对路径（<code>data/appstore/logs/run-&lt;run_id&gt;.log</code>）</td></tr>
              <tr><td><code>CPU_NUM</code></td><td>可用 CPU 核数（编译可参考，如 make -j）</td></tr>
              <tr><td><code>PKG_PATH</code></td><td>本次运行脚本快照目录（含 app.yaml / 脚本 / options.env / options.json）</td></tr>
              <tr><td><code>PKG_SRC_PATH</code></td><td>仓库内源码目录（<code>repos/&lt;repo&gt;/&lt;category&gt;/&lt;name&gt;</code>），需要读源码附件时用</td></tr>
              <tr><td><code>APP_FAMILY</code></td><td>目标版本所属家族（如 mysql / mariadb）。仅当 <code>app.yaml version_meta</code> 声明了该版本的 family 时注入；install / uninstall / upgrade 均会注入。脚本家族分流的唯一依据，<strong>不要按版本号自行猜测</strong>（跨家族版本号可能重合）</td></tr>
              <tr><td><code>APP_ID</code></td><td>本次运行 run_id</td></tr>
              <tr><td><code>APP_NAME</code></td><td>包名</td></tr>
              <tr><td><code>APP_PATH</code></td><td>本应用元数据登记目录（<code>$ZAP_PATH/data/apps/&lt;category&gt;/&lt;name&gt;</code>）：系统写 <code>meta.yaml</code>、脚本登记 <code>info.yaml</code>，勿放安装产物（登记字段见第三节「实例登记」）</td></tr>
              <tr><td><code>BUILD_PATH</code></td><td>本次运行专属编译目录（<code>$ZAP_PATH/data/appstore/runs/&lt;run_id&gt;/build</code>），编译中间产物放这里；脚本开头可放心 <code>rm -rf</code>——路径按 run 隔离，成功后随运行现场一并清理、失败保留供排查</td></tr>
              <tr><td><code>ZAP_DATA_PATH</code></td><td>面板数据目录（<code>$ZAP_PATH/data</code>）</td></tr>
              <tr><td><code>APP_VERSION</code></td><td>本次安装/升级的目标版本</td></tr>
              <tr><td><code>MAJOR_VERSION</code></td><td>目标版本主版本号（如 1.24.0 → 1）</td></tr>
              <tr><td><code>MINOR_VERSION</code></td><td>目标版本次版本号（如 1.24.0 → 24）</td></tr>
              <tr><td><code>APP_OLD_VERSION</code></td><td>升级前旧版本（仅升级注入）</td></tr>
              <tr><td><code>ACTION</code></td><td>动作键（由 actions 自定义操作发起时注入，如 build）</td></tr>
              <tr><td><code>ZAP_USER</code></td><td>发起本次操作的面板登录用户名（install / uninstall / upgrade 及失败重跑都会注入）。多用户 / 多角色场景下，脚本可据此把安装产物、文件属主等归属到操作者名下</td></tr>
              <tr><td><code>ZAP_RUN_MODE</code></td><td>虚拟主机运行模式：恒为 <code>system</code>——每个面板用户一个独立 Linux 账号（nologin），站点文件与运行身份均归该账号（历史上一度支持 <code>www</code> 统一用户，现已移除）</td></tr>
              <tr><td><code>ZAP_LINUX_USER</code></td><td>降权运行（<code>run_as: user</code>）时的 Linux 账号名；非降权包不注入</td></tr>
              <tr><td><code>ZAP_HOME</code></td><td>降权运行时该账号的家目录（进程 <code>cwd</code> 也在此），脚本产物应落在它下面</td></tr>
              <tr><td><code>ZAP_PY_LIB</code></td><td>Python 辅助库目录（<code>$ZAP_PATH/scripts/zap</code>），供 <code>sys.path.insert</code> 后 <code>import zapweb</code>；仅 Python 脚本注入</td></tr>
              <tr><td><code>SITE_ID / SITE_DOMAIN</code></td><td>目标站点 ID 与主域名（<code>provision.site</code> 编排后注入）</td></tr>
              <tr><td><code>SITE_ROOT</code></td><td>站点文档根目录（脚本的部署目标，越界写入会被属主拦住）</td></tr>
              <tr><td><code>SITE_OWNER / SITE_LINUX_USER</code></td><td>站点归属的面板用户名与 Linux 账号名</td></tr>
              <tr><td><code>PHP_INSTANCE / PHP_FPM_SOCK</code></td><td>站点绑定的 PHP 实例（<code>php83</code>）与专属 FPM 通道（<code>unix:/var/run/php-fpm-&lt;用户&gt;-83.sock</code>）</td></tr>
              <tr><td><code>DB_NAME / DB_USER / DB_PASS</code></td><td>面板为本实例建的专用库、专用用户与随机密码（密码只在脚本 env 与 <code>provision.json</code>（0600）出现，前端拿不到）</td></tr>
              <tr><td><code>DB_HOST / DB_PORT / DB_CHARSET / DB_USER_HOST</code></td><td>连接地址、端口、字符集，以及该账号被授权的连接来源（默认 <code>localhost</code>）</td></tr>
              <tr><td>选项变量</td><td>每个 options 项按 <code>name</code> 直接注入同名环境变量（见第七节）</td></tr>
            </tbody>
          </table>

          <!-- 六、options 定义 -->
          <h2 id="sec-options">六、options：安装 / 升级 / 卸载可选项</h2>
          <p>
            <code>app.yaml</code> 顶层 <code>options</code> 可按动作键（<code>install</code> / <code>upgrade</code> / <code>uninstall</code>）分别定义；
            Web 端检测到选项时，点击对应按钮会先弹出选项表单，确认后选项随该动作请求提交。
            动作键的值支持两种写法：<strong>选项数组</strong>，或 <strong><code>{ items, intro }</code> 对象</strong>——
            其中 <code>items</code> 为选项数组，<code>intro</code> 为整组介绍 / 说明（纯展示，不参与提交与 env 注入），
            Web 端渲染在<strong>选项表单最下方</strong>；仅有 <code>intro</code> 而无 <code>items</code> 时同样会弹出，作为该动作的说明页。
            install / upgrade 未声明动作键时缺省回退 <code>install</code> 定义；uninstall 不回退（未声明 <code>options.uninstall</code> 即不弹窗）。
            顶层直接写数组等价于作用于全部动作：
          </p>
          <pre class="code">{{ codes.optionsYaml }}</pre>
          <table class="doc-table">
            <thead>
              <tr><th style="width: 160px">字段</th><th style="width: 120px">类型</th><th>说明</th></tr>
            </thead>
            <tbody>
              <tr><td><code>name</code></td><td>string</td><td>选项名，即注入的环境变量名 / options.env 键（命名规则见第八节）</td></tr>
              <tr><td><code>label</code></td><td>string</td><td>表单显示名</td></tr>
              <tr><td><code>type</code></td><td>string</td><td>控件类型：<code>string</code>（文本）/ <code>number</code>（数值）/ <code>bool</code>（开关）/ <code>select</code>（单选）/ <code>multiselect</code>（多选）；缺省 string</td></tr>
              <tr><td><code>default</code></td><td>any</td><td>缺省值</td></tr>
              <tr><td><code>required</code></td><td>bool</td><td>是否必填</td></tr>
              <tr><td><code>placeholder</code></td><td>string</td><td>string / select 输入框占位提示</td></tr>
              <tr><td><code>desc</code></td><td>string</td><td>字段下方说明文字</td></tr>
              <tr><td><code>choices</code></td><td>array</td><td>select / multiselect 候选：字符串，或 <code>{ label, value }</code> 对象（label 为显示名）</td></tr>
              <tr><td><code>separator</code></td><td>string</td><td>multiselect 提交时的拼接符，缺省为空格；候选值本身可能含空格时建议改用其它分隔符</td></tr>
            </tbody>
          </table>
          <p><strong>值归一化</strong>（提交后全部为标量字符串，无 JSON 对象）：</p>
          <ul>
            <li><code>bool</code> → <code>true</code> / <code>false</code>；</li>
            <li><code>number</code> → 数字字符串；</li>
            <li><code>multiselect</code> → 勾选项按 <code>separator</code> 拼接为一个字符串（缺省空格，如 <code>"ssl gzip stub_status"</code>）；</li>
            <li>提交后选项随该动作落盘并注入运行环境变量，各动作的选项互不干扰（升级缺省策略为「先卸载再安装」时，升级选项对两个脚本均可见）。</li>
          </ul>

          <!-- 七、脚本如何读取 -->
          <h2 id="sec-read">七、脚本如何读取选项（核心）</h2>
          <p>选项会随快照一起落盘，并与脚本放在同一个目录，脚本有三种等价读取方式，任选其一：</p>
          <pre class="code">{{ codes.readOpts }}</pre>
          <ul>
            <li><strong>环境变量（推荐）</strong>：每个选项按 <code>name</code> 直接注入子进程 env，脚本零成本使用 <code>$MODULES</code> 即可，无需 source；</li>
            <li><strong>options.env</strong>：快照目录内每行 <code>NAME='值'</code>（单引号转义，可安全 source），也便于在「编辑脚本 / 重跑」前人工查看与修改；</li>
            <li><strong>options.json</strong>：结构化 JSON 对象，适合 python / jq 等程序化处理。</li>
          </ul>
          <el-alert type="info" :closable="false" class="doc-tip">
            多选值默认以空格拼接。脚本中如需逐项遍历：<code>for m in $MODULES; do ...; done</code>；
            若包声明了 <code>separator: ','</code>，可先 <code>MODULES=${MODULES//,/ }</code> 再遍历。
          </el-alert>

          <!-- 八、校验与限制 -->
          <h2 id="sec-limits">八、选项命名与长度限制</h2>
          <table class="doc-table">
            <thead>
              <tr><th style="width: 220px">维度</th><th>限制</th></tr>
            </thead>
            <tbody>
              <tr><td>选项名</td><td>≤ 64 字符；首字符须英文字母或下划线；其余仅 ASCII 字母 / 数字 / 下划线</td></tr>
              <tr><td>保留前缀 / 名称</td><td>禁止 <code>ZAP_</code> / <code>PKG_</code> / <code>APP_</code> / <code>ACTION</code> / <code>SCRIPT_</code> / <code>RUN_</code> 前缀，禁止 <code>PATH</code> / <code>HOME</code></td></tr>
              <tr><td>单次数量</td><td>≤ 64 项</td></tr>
              <tr><td>单值长度</td><td>≤ 4096 字符（按 Unicode 字符数计，含多选拼接后的整体值）</td></tr>
            </tbody>
          </table>
          <p><strong>关于「env 长度」的系统层限制</strong>：选项值最终随单个环境变量经 <code>execve</code> 传给 bash，因此存在两层系统硬限制——单个环境变量串（<code>KEY=VALUE</code>）上限 <code>MAX_ARG_STRLEN</code> = 128KB，全部 env + 参数合计上限 <code>ARG_MAX</code>（通常 2MB，<code>getconf ARG_MAX</code> 可查）。超出会直接报 <code>E2BIG</code>。由于应用层已把单值限制在 4096 字符（全中文 ≈ 12KB、64 项全满也远低于 2MB），正常 Web 表单路径不会触达系统限制，实际瓶颈是 4096。</p>
          <el-alert type="warning" :closable="false" class="doc-tip">
            两点提醒：① 4096 按「字符」计数、execve 按「字节」计数，极端全 emoji（4 字节/字符）单选项也只有 16KB，安全；② 「编辑脚本 / 重跑」路径直接读取
            <code>options.env</code> 文件、不做 4096 长度校验，手工往文件里塞超长值（单行超过 128KB）会在启动 bash 时 <code>E2BIG</code> 失败，请勿这样做。
          </el-alert>

          <!-- 九、完整示例 -->
          <h2 id="sec-example">九、完整示例：nginx 编译模块多选</h2>
          <p>仓库样例 <code>infra/nginx</code>（数据目录 <code>data/appstore/repos/zap-appstore/infra/nginx/</code>）演示了「编译哪些模块」的多选场景。要点：</p>
          <ul>
            <li>动作键 <code>build</code> 与 <code>actions.build: 编译安装</code> 对应，从该动作发起安装时用户可勾选模块；</li>
            <li>选项脚本直接以 <code>$MODULES</code> / <code>$EXTRA_CONFIG</code> 取用（env 已注入）；</li>
            <li>多选缺省空格拼接，configure 尾部整体展开即得到模块参数。</li>
          </ul>
          <pre class="code">{{ codes.nginxYaml }}</pre>
          <pre class="code">{{ codes.nginxUse }}</pre>

          <!-- 十、合并入口示例 -->
          <h2 id="sec-family">十、完整示例：MySQL / MariaDB 合并入口</h2>
          <p>
            <strong>合并入口</strong>指一个包承载多个「家族」（同族不同系列），典型是官方样例
            <code>database/mysql</code>：同一入口安装 MySQL 或 MariaDB，两者一次只能安装其一。
            家族的唯一事实来源是 <code>app.yaml version_meta</code>——前端分组展示、zapexec
            下发家族、脚本分流全部以它为准，脚本不得自行按版本号猜测家族。
          </p>
          <p class="sec-sub"><strong>为什么不能按版本号猜家族？</strong></p>
          <ul>
            <li>家族与版本是「一一映射」：<code>version_meta</code> 以版本字符串为键，<strong>同一包内版本号必须全局唯一</strong>——若未来 MySQL 与 MariaDB 出现完全相同的版本串，该版本无法同时归属两家，只能拆成独立入口或错开版本号；</li>
            <li>即便版本串不重复（如 mysql 9.x 与 mariadb 9.x 并存），仅按主版本号（8 / 9 / 10…）粗分也会产生歧义；</li>
            <li>安装装错家族代价高（安装目录 / 服务单元 / 配置全不同），取不到家族宁可报错也不猜。</li>
          </ul>
          <pre class="code">{{ codes.familyYaml }}</pre>
          <p>
            <code>version_meta</code> 条目字段：<code>family</code> = 家族标识（分组、校验与 env 下发用）；
            <code>label</code> = 家族短名（版本下拉选项内标签、卡片选中态标签，如 MySQL / MariaDB）；
            <code>group</code> = 下拉分组标题（如 MySQL Community Server / MariaDB Server）。
            <strong>展示名（label / group）无任何前端内置映射</strong>，全由这里声明——新增家族只需改 app.yaml，无需改前端。
          </p>
          <p class="sec-sub"><strong>脚本侧：只认系统下发的 APP_FAMILY</strong></p>
          <p>
            install / uninstall / upgrade 发起时，zapexec 以所选版本在 <code>version_meta</code>
            反查家族并注入环境变量 <code>APP_FAMILY</code>（见第五节），入口脚本据此直接分流到对应家族子脚本：
          </p>
          <pre class="code">{{ codes.familyInstall }}</pre>
          <ul>
            <li>入口脚本只做分流，真实安装逻辑放在 <code>mysql-install.sh</code> / <code>mariadb-install.sh</code> 等家族子脚本内（文件名自定，入口以 <code>exec</code> 转交，子脚本保持 <code>set -euo pipefail</code>）；</li>
            <li><code>APP_FAMILY</code> 缺失（该版本未在 version_meta 声明）时，安装入口应<strong>直接报错退出</strong>而非猜测；卸载脚本可回退按主版本号猜测并打印告警（旧运行环境兜底）；</li>
            <li>新增版本须同步维护 <code>version</code> 数组与 <code>version_meta</code> 条目，缺一不可；</li>
            <li>合并入口不建议单独写 <code>upgrade.sh</code>：升级走系统默认两段式（先卸载旧家族、再安装目标版本），天然规避跨家族残留（MySQL → MariaDB 等场景）。</li>
          </ul>

          <!-- 十一、失败排查 -->
          <h2 id="sec-trouble">十一、失败排查与重跑工作流</h2>
          <ol>
            <li>「应用商店 → 运行记录」查看失败运行的日志（末尾 <code>__ZAP_DONE__ &lt;code&gt;</code> 即退出码）；</li>
            <li>失败现场默认保留在 <code>data/appstore/runs/&lt;run_id&gt;/</code>：<code>pkg/</code> 内脚本与 <code>options.env</code> 可查看/编辑，<code>build/</code> 编译残留一并保留供排查；然后「编辑脚本 / 重跑」——重跑以快照内文件为准，编辑过的选项同样生效；</li>
            <li>重跑成功后系统按新 run_id 重新跟踪日志；新运行成功会清理其快照，原失败快照由「重跑」发起时一并清理。</li>
          </ol>
          <!-- 十二、运行身份与脚本语言 -->
          <h2 id="sec-runas">十二、运行身份与脚本语言</h2>
          <p>默认所有包脚本以 <strong>root</strong> 执行。建站类包（<code>webapps</code>，如 WordPress / Typecho）可声明降权，让脚本以<strong>面板用户对应的 Linux 账号</strong>（nologin）运行：</p>
          <pre class="code-block">category: webapps
scope: site                # 作用范围：site = 装进用户站点（缺省即降权）；panel = 面板级工具（如 phpMyAdmin）
run_as: user               # 显式声明运行身份，优先级高于 scope；只允许 webapps 分类声明
scripts:
  install: install.py      # .py 自动用 python3 执行；.sh 用 bash
  uninstall: { file: remove.py, interpreter: python3 }</pre>
          <ul>
            <li><strong>门禁</strong>：非 <code>webapps</code> 分类声明 <code>run_as: user</code> 会被面板与 zapexec 双重拒绝——降权通道只为「装进站点目录」的建站包开放。</li>
            <li><strong>降权后拿不到的能力</strong>：不能写 <code>/usr/local/apps</code>、不能改 nginx 配置、不能碰 systemd、拿不到数据库管理凭据。建站所需的「建站点 / 建库」由面板侧完成并以环境变量注入，脚本只负责<strong>下载、落地、写配置</strong>。</li>
            <li><strong>可写范围</strong>：家目录（<code>$ZAP_HOME</code>）、<code>$BUILD_PATH</code>、<code>$APP_PATH</code>（系统已预建并 chown 给该账号）；越界会被文件系统属主直接拦住。</li>
            <li><strong>环境</strong>：进程 <code>env_clear()</code> 后只注入白名单，PATH 不含 sbin；<code>cwd</code> 为家目录；<code>NO_NEW_PRIVS</code>（禁止借 setuid 再提权）+ 关 core dump（避免内存里的数据密码落盘）。</li>
          </ul>
          <p class="sec-sub"><strong>脚本可以用 python3 写</strong>：扩展名 <code>.py</code> 或 <code>interpreter: python3</code> 时以 <code>python3 -I -B &lt;脚本&gt;</code> 执行。</p>
          <ul>
            <li><code>-I</code> 隔离模式：忽略 <code>PYTHON*</code> 环境变量、不加载用户 site-packages、<strong>不把脚本目录放进 <code>sys.path</code></strong>；因此引入辅助库必须显式加路径：</li>
          </ul>
          <pre class="code-block">import os, sys
sys.path.insert(0, os.environ["ZAP_PY_LIB"])   # 注入的辅助库目录
from zapweb import log_info, log_ok, download, extract, deploy, render, write_info

log_info("开始安装")
archive = download(os.environ["PKG_URL"], os.path.join(os.environ["BUILD_PATH"], "app.tar.gz"))</pre>
          <ul>
            <li><code>-B</code>：不生成 <code>__pycache__</code>，家目录不留可执行字节码。</li>
            <li><code>zapweb</code> 提供与 <code>bash_utils.sh</code> 等价的能力：<code>download(url, dest, sha256=...)</code>（给了摘要就强制校验）、<code>extract</code> / <code>deploy</code>（含路径围栏 <code>assert_under</code>）、<code>render</code>（<code v-pre>{{NAME}}</code> 占位替换，不做 shell 展开）、<code>write_info</code>（拒绝写入含 PASS/SECRET/TOKEN 的字段）、<code>mask()</code>（密码打码后再进日志）。</li>
            <li>需要第三方库时请在脚本内显式安装到用户目录（<code>pip install --user</code>），并在 <code>app.yaml</code> 的 <code>dependencies</code> 里声明 <code>python3</code> 版本要求。</li>
          </ul>
          <!-- 十三、建站与建库编排 -->
          <h2 id="sec-provision">十三、建站 / 建库编排（provision）</h2>
          <p>降权脚本（<code>run_as: user</code>）拿不到「建站点」「建数据库」的权限，也绝不应当拿到数据库管理凭据。因此这两件事由<strong>面板在任务入队前完成</strong>，再把结果注入脚本环境：</p>
          <pre class="code-block">provision:
  site:
    mode: create            # create（缺省，不存在就建）/ require（站点必须已存在）
    domain_option: SITE_DOMAIN   # 从安装选项里取域名的选项名
    php: php83              # PHP 实例，缺省取面板默认 PHP
    rewrite: wordpress      # 伪静态预设
  database:
    name: wp                # 库名基名，实际库名 = 用户前缀 + wp（重名自动 wp_2、wp_3…）
    charset: utf8mb4
    host: localhost         # 专用账号被授权的连接来源
options:
  install:
    - { name: SITE_DOMAIN, label: 站点域名, type: text, default: '', required: true }</pre>
          <p class="sec-sub"><strong>执行顺序</strong>（<code>zapd</code> 的 <code>provision_for()</code>，同步执行；任一步失败就不入队，不会出现「脚本跑一半发现没库」）：</p>
          <ol>
            <li><strong>站点</strong>：按域名在操作者管理范围内查找 → 已存在直接复用（同一域名重复安装不会建出第二个站）；不存在时按 <code>/site/add</code> 的同一套规则新建（套餐站点数配额 → 域名唯一性 → 目录规划 → 档案 → vhost 同步），PHP 实例缺省取 <code>php_default</code>。</li>
            <li><strong>数据库</strong>：走与「数据库 → 新建」<strong>完全同一个函数</strong>（<code>database::create_schema</code>）——配额校验、非 admin 自动加 <code>{用户名}_</code> 前缀、随机 16 位密码、建用户失败回滚刚建的库；重名自动加序号。</li>
            <li><strong>注入</strong>：结果写入 <code>data/apps/&lt;包路径&gt;/provision.json</code>（0600，仅 root）并以环境变量注入脚本：<code>SITE_ID / SITE_DOMAIN / SITE_ROOT / SITE_OWNER / SITE_LINUX_USER / PHP_INSTANCE / PHP_FPM_SOCK</code> 与 <code>DB_NAME / DB_USER / DB_PASS / DB_HOST / DB_PORT / DB_CHARSET / DB_USER_HOST</code>。</li>
            <li><strong>执行</strong>：脚本（Linux 账号）只做下载 → 解压到 <code>$SITE_ROOT</code> → 写配置文件 → 登记 <code>info.yaml</code>。</li>
            <li><strong>重跑 / 卸载</strong>：重跑复用 <code>run.json</code> 里记录的同一套站点与库（<strong>不会重复建库</strong>）；卸载把 <code>provision.json</code> 回传给 <code>uninstall.sh</code>，脚本可先 <code>mysqldump</code> 备份再删文件。</li>
          </ol>
          <p class="sec-sub"><strong>典型脚本（Python）</strong>：</p>
          <pre class="code-block">import os, sys
sys.path.insert(0, os.environ["ZAP_PY_LIB"])
from zapweb import *

root = env_required("SITE_ROOT")          # 面板给的站点根目录
archive = download(env_required("PKG_URL"), tmp_dir() / "wp.tar.gz")
src = single_subdir(extract(archive, env("BUILD_PATH")))
deploy(src, root, root)                   # 第二、三个参数是路径围栏，越界即中止

render(os.path.join(env_required("PKG_SRC_PATH"), "wp-config.php.tpl"),
       os.path.join(root, "wp-config.php"),
       {"DB_NAME": env("DB_NAME"), "DB_USER": env("DB_USER"),
        "DB_PASSWORD": env("DB_PASS"), "DB_HOST": env("DB_HOST")})
# 密码绝不进 info.yaml（write_info 会拒绝 *_PASS / *_SECRET 类字段），也绝不 log
write_info(env_required("APP_PATH"), domain=env("SITE_DOMAIN"),
           site_root=root, version=env("APP_VERSION"), db=env("DB_NAME"))</pre>
          <ul>
            <li><strong>完整样板</strong>：<code>data/appstore/repos/zap-appstore/webapps/wordpress/</code>（<code>app.yaml</code> + <code>install.py</code> + <code>uninstall.py</code> + <code>upgrade.py</code> + <code>wp-config.php.tpl</code>），可直接照抄结构改自己的包。</li>
            <li><strong>卸载默认不删库</strong>：库里可能有用户数据，脚本可在 <code>uninstall.sh</code> 里自行决定是否 <code>drop</code>（已回传 <code>DB_*</code>）。</li>
            <li><strong>不要尝试自己建库</strong>：脚本没有 <code>zapadm</code> 凭据，也不要把凭据写进包里——那会绕过套餐配额与多租户隔离。</li>
            <li><strong>密码处理</strong>：只从 <code>$DB_PASS</code> 读取、写进配置文件后不再出现；日志里用 <code>mask()</code>；<code>wp-config.php</code> 建议 <code>chmod 640</code>。</li>
          </ul>
          <!-- 十四、bash_utils 公共函数库 -->
          <h2 id="sec-utils">十四、bash_utils.sh 公共函数库</h2>
          <p>
            bash 脚本统一在开头 source 公共库即可使用下列函数（纯函数库：被 source 时不改动调用方 shell 选项、不强制退出，
            Python 脚本请用等价的 <code>zapweb</code>，见第十二节）：
          </p>
          <pre class="code-block">#!/bin/bash
set -euo pipefail
source "${ZAP_PATH}/scripts/zap/bash_utils.sh"</pre>
          <p>source 后顶层变量立即可用：<code>OS_NAME</code>（发行版小写 ID）、<code>OS_VERSION</code>、<code>OS_ID_LIKE</code>、<code>OS_PRETTY</code>、<code>OS_ARCH</code>、<code>OS_ARCH_ALIAS</code>、<code>OS_MACHINE</code>；可随时重跑 <code>os_detect</code> 刷新。</p>

          <p class="sec-sub"><strong>① 日志与前置（用户 / 目录 / 系统依赖）</strong></p>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>log_info</code> / <code>log_ok</code> / <code>log_warn</code> / <code>log_error</code></td><td>统一日志格式（<code>[时间] [级别]</code>，warn / error 走 stderr），实时写入运行日志。用法：<code>log_info "开始编译 ${APP_VERSION}"</code></td></tr>
              <tr><td><code>assert_root</code></td><td>非 root 返回 1（不会自己 exit）。用法：<code>assert_root || { log_error "需要 root"; exit 1; }</code></td></tr>
              <tr><td><code>ensure_dir &lt;dir&gt; [...]</code></td><td>建目录（幂等，可一次传多个），任一失败返回 1。用法：<code>ensure_dir "${BUILD_PATH}" "${PKG_PATH}"</code></td></tr>
              <tr><td><code>ensure_group &lt;group&gt;</code></td><td>确保系统组存在（<code>groupadd -r</code>，Alpine 走 <code>addgroup</code>），已存在直接返回 0</td></tr>
              <tr><td><code>ensure_user &lt;user&gt; [group...]</code></td><td>确保运行用户存在；组不存在先建再把用户加进去（用户已存在时也会补齐附加组）。用法：<code>ensure_user mysql mysql</code>、<code>ensure_user www www zap</code></td></tr>
              <tr><td><code>ensure_usergroup &lt;user&gt; &lt;group&gt; [...]</code></td><td>只把<strong>已存在</strong>的用户加入组：用户不存在直接报错，避免拼错用户名被静默创建。用法：<code>ensure_usergroup "${U}" docker</code></td></tr>
              <tr><td><code>prepare_install_env [user] [group...]</code></td><td>前置汇总：运行用户 + 关键目录（<code>PKG_PATH</code> / <code>BUILD_PATH</code>）+ 首次系统编译依赖。缺省 <code>www www</code>，组缺省与用户同名。依赖<strong>装全了才写锁</strong>（<code>system_deps.lock</code>），没装全不写锁 → 下次运行重试；<code>ZAP_FORCE_DEPS=1</code> 可强制重装。用法：<code>prepare_install_env www</code>、<code>prepare_install_env mysql</code></td></tr>
              <tr><td><code>install_system_deps</code></td><td>按发行版批量装编译依赖（<code>UBUNTU_DEPS</code> / <code>RH_DEPS</code> / <code>ALPINE_DEPS</code>，均可用 <code>ZAP_</code> 前缀同名环境变量整体覆盖）；批量失败自动逐项补装，返回 0 = 全部就绪。一般不直接调用，走 <code>prepare_install_env</code></td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>② 系统 / 版本 / 包管理</strong></p>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>os_detect</code></td><td>探测发行版 / 内核 / 架构并刷新顶层变量（source 时已自动跑过一次，改过 <code>/etc/os-release</code> 可重跑）</td></tr>
              <tr><td><code>is_os &lt;id...&gt;</code></td><td>匹配 ID <strong>或</strong> ID_LIKE：<code>is_os ubuntu debian</code>；因 Ubuntu 的 ID_LIKE 含 debian，<code>is_os debian</code> 在 Ubuntu 上也为真</td></tr>
              <tr><td><code>is_os_strict &lt;id&gt;</code></td><td>只匹配 ID，不认 ID_LIKE（要严格区分 Ubuntu / Debian 时用）：<code>is_os_strict debian</code></td></tr>
              <tr><td><code>os_version_major</code></td><td>系统主版本号（stdout 输出）：<code>24.04 → 24</code>、<code>7.9 → 7</code>。用法：<code>[ "$(os_version_major)" -ge 24 ]</code></td></tr>
              <tr><td><code>os_version_ge &lt;ver&gt;</code> / <code>os_version_lt &lt;ver&gt;</code></td><td>与当前系统版本比较：<code>os_version_ge 24.04</code></td></tr>
              <tr><td><code>is_os_ge &lt;id&gt; &lt;ver&gt;</code></td><td>发行版 + 版本下限<strong>同时</strong>成立：<code>is_os_ge ubuntu 24.04</code>（沿用 <code>is_os</code> 的 ID_LIKE 语义；只认 ID 请用 <code>is_os_strict</code> + <code>os_version_ge</code>）</td></tr>
              <tr><td><code>is_deb_family</code> / <code>is_rpm_family</code></td><td>deb 系（ubuntu / debian / mint / …）/ rpm 系（rhel / rocky / alma / fedora / …）</td></tr>
              <tr><td><code>pkg_manager</code></td><td>输出 <code>apt</code> / <code>dnf</code> / <code>yum</code> / <code>apk</code> / <code>zypper</code>，未识别返回 1（只认 Linux 发行版的包管理器；部分 Linux 上也有叫 <code>pkg</code> 的零散命令，故不看命令名直接判定）。用法：<code>PKG_MGR="$(pkg_manager || true)"</code></td></tr>
              <tr><td><code>normalize_arch [arch]</code></td><td>架构归一化：<code>x86_64 → amd64</code>、<code>aarch64 → arm64</code>（缺省取 <code>uname -m</code>）</td></tr>
              <tr><td><code>cpu_count</code></td><td>可用核数（受注入的 <code>CPU_NUM</code> 上限约束，探测失败回退 1）</td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>③ 运行时库 / 系统包（跨发行版差异）</strong></p>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>have_lib &lt;soname|glob&gt;</code></td><td>系统里是否已有该库：先查 ldconfig 缓存，缓存未收录时再按默认目录复核文件。按 soname <strong>精确</strong>匹配：<code>have_lib libaio.so.1</code> 不认 <code>libaio.so.1t64</code>；也可给通配 <code>have_lib 'libncurses.so.*'</code></td></tr>
              <tr><td><code>lib_path &lt;soname|glob&gt;</code></td><td>取库的实际路径（缓存优先，再按默认目录找），未找到返回 1。用法：<code>src="$(lib_path libaio.so.1t64)"</code></td></tr>
              <tr><td><code>link_lib_compat &lt;需要的 soname&gt; &lt;现有 soname&gt;</code></td><td>发行版改了库文件名、官方二进制仍按旧 soname 加载时补同名软链 + 刷新缓存（幂等，已存在直接返回 0）。用法：<code>link_lib_compat libaio.so.1 libaio.so.1t64</code></td></tr>
              <tr><td><code>pkg_install_any &lt;pm&gt; &lt;候选包名...&gt;</code></td><td>依次尝试候选包名，装上任意一个即成功；全失败返回 1 并把包管理器的错误写进日志（不中断脚本，后果由调用方决定）。apt 走 <code>DEBIAN_FRONTEND=noninteractive</code> + <code>--no-install-recommends</code>；apk 走 <code>apk add</code>。用法：<code>pkg_install_any apt libaio1t64 libaio1</code>、<code>pkg_install_any apk png</code></td></tr>
              <tr><td><code>ldconfig_bin</code> / <code>lib_search_dirs</code></td><td>内部辅助（一般不必直接调用）：定位 <code>ldconfig</code>（它在 <code>/sbin</code>，守护进程 PATH 里常没有）/ 列出动态链接器默认搜索目录</td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>④ 下载 / 解压 / 编译</strong></p>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>fetch_file &lt;url&gt; &lt;dest&gt; [重试次数]</code></td><td>下载：curl 优先、wget 回退、自动重试（默认 3 次），非 TTY 下也输出单行进度条；失败返回 1（不中断脚本）</td></tr>
              <tr><td><code>download_file &lt;url&gt; &lt;dest&gt;</code></td><td>下载并打日志，失败<strong>直接 exit 1</strong>（适合「下不到就没必要继续」的场景）</td></tr>
              <tr><td><code>http_fetch &lt;url&gt; &lt;dest&gt;</code></td><td><code>download_file</code> 的旧名，行为一致</td></tr>
              <tr><td><code>extract_archive &lt;归档&gt; [目标目录]</code></td><td>按扩展名解压（<code>.tar.gz</code> / <code>.tgz</code> / <code>.tar.xz</code> / <code>.tar.bz2</code> / <code>.tar</code> / <code>.zip</code>），目标目录缺省为当前目录</td></tr>
              <tr><td><code>download_extract &lt;url&gt; &lt;本地归档名&gt; &lt;目标目录&gt;</code></td><td>下载 + 解压一步到位，任一步失败返回 1</td></tr>
              <tr><td><code>MakeInstall [并行数]</code></td><td>先由 <code>make_bin</code> 挑出 GNU make（系统自带的未必是 GNU make，不认 GNU Makefile 时须用 <code>gmake</code>；可用 <code>ZAP_MAKE</code> 覆盖，值不自报 GNU Make 的会被跳过），再 <code>-jN</code> 并安装，失败自动退回串行；并行数缺省 <code>CPU_NUM</code> → <code>cpu_count</code>。用法：<code>./configure --prefix="${APP_PATH}" &amp;&amp; MakeInstall</code></td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>⑤ 版本比较 / 解析</strong></p>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>version_compare &lt;a&gt; &lt;b&gt;</code></td><td>返回 0 相等 / 1 a&gt;b / 2 a&lt;b（点分数字，忽略字母后缀：<code>1.24.0p1</code> 视作 <code>1.24.0</code>）</td></tr>
              <tr><td><code>version_ge</code> / <code>version_gt</code> / <code>version_lt</code></td><td>基于 <code>version_compare</code> 的快捷判断。用法：<code>version_ge "${APP_VERSION}" "8.4" &amp;&amp; ...</code></td></tr>
              <tr><td><code>version_field &lt;ver&gt; &lt;段号&gt;</code></td><td>取第 N 段（从 1 起）：<code>version_field 1.1.1w 3</code> → <code>1</code></td></tr>
              <tr><td><code>version_major &lt;ver&gt;</code> / <code>version_minor &lt;ver&gt;</code></td><td>主 / 次版本号：<code>version_major 1.1.1w</code> → <code>1</code></td></tr>
              <tr><td><code>version_major_minor &lt;ver&gt;</code></td><td>一次取主次版本，输出 <code>"major minor"</code>。用法：<code>read -r MAJOR MINOR &lt;&lt;&lt;"$(version_major_minor "${APP_OLD_VERSION}")"</code></td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>⑥ 配置读写 / 路径安全</strong></p>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>random_password [长度]</code></td><td>生成随机密码（默认 16 位字母数字；<code>openssl</code> → <code>/dev/urandom</code> → <code>sha256sum</code> 三级回退）。用法：<code>DB_PASS="$(random_password 20)"</code></td></tr>
              <tr><td><code>has_git</code></td><td>系统是否有 git：<code>has_git &amp;&amp; git clone ...</code></td></tr>
              <tr><td><code>getPropsValue &lt;文件&gt; &lt;key&gt;</code></td><td>读 <code>key=value</code> 属性文件里的值（允许行内注释后的值），无匹配返回空</td></tr>
              <tr><td><code>yaml_value &lt;文件&gt; &lt;key&gt;</code></td><td>读 YAML <strong>顶层</strong> <code>key: value</code>（只支持简单形式），无匹配返回空</td></tr>
              <tr><td><code>normalize_dir &lt;path&gt;</code></td><td>去掉末尾多余斜杠（保留根 <code>/</code>，不解析软链），空路径返回 1</td></tr>
              <tr><td><code>resolve_install_dir &lt;info.yaml&gt; &lt;软链&gt; [键名]</code></td><td>确定安装目录：info 文件里登记的值优先，其次软链指向。用法：<code>resolve_install_dir "${APP_PATH}/info.yaml" "${APPS_DIR}/phpmyadmin"</code></td></tr>
              <tr><td><code>assert_under_apps_dir &lt;目标&gt; [应用根目录]</code></td><td>路径围栏：目标必须是应用根目录的<strong>直接子目录</strong>，否则拒绝（防误删）；应用根目录缺省取 <code>APPS_DIR</code></td></tr>
              <tr><td><code>path_under &lt;path&gt; &lt;prefix&gt;</code></td><td>判断路径是否在指定前缀下（删除前的围栏）：<code>path_under "${APPS_DIR}/mysql-8.0" "${APPS_DIR}"</code></td></tr>
              <tr><td><code>wzap_conf &lt;key&gt; &lt;value&gt;</code></td><td>写 <code>/root/zap.conf</code> 的 <code>key=value</code>（已存在则覆盖）；文件可用 <code>WZAP_CONF_FILE</code> 覆盖，key 仅允许字母数字下划线</td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>依赖安装的正确姿势</strong></p>
          <el-alert type="warning" :closable="false" class="doc-tip">
            两条硬规则：① 不要写 <code>apt-get install -y &lt;一长串&gt; || true</code>——apt 只要有一个包名找不到就整条命令失败、
            <strong>一个都不装</strong>，再被 <code>|| true</code> 吞掉，问题会推迟到编译 / 启动阶段以难懂的报错暴露；
            ② 不要只按包名判断依赖是否就绪——同一库在不同版本里 soname 可能已改名，要用 <code>have_lib</code> 按 soname 复核。
          </el-alert>
          <pre class="code">{{ codes.runtimeDeps }}</pre>

          <el-alert type="danger" :closable="false" class="doc-tip">
            <strong>踩过的坑：</strong><code>ldconfig -p</code> 每行以 <strong>Tab 开头</strong>，用 <code>${line%% *}</code> 取 soname 会把 Tab 留在名字里
            （<code>"\tlibaio.so.1t64"</code>），于是永远匹配不上——库明明装着却判成「缺失」，进而误报「缺少 libaio.so.1」。
            正确做法是用 <code>read -r name rest</code> 分词（自动吃掉前导空白）。另外 <code>ldconfig</code> 在 <code>/sbin</code>，
            守护进程拉起的脚本 PATH 里常常没有，直接写 <code>ldconfig -p</code> 会「命令未找到」→ 同样误判成缺库。
          </el-alert>

          <p class="sec-sub"><strong>安装残局与失败重跑</strong></p>
          <p>
            系统只在脚本<strong>成功退出</strong>后才写 <code>apps/&lt;cat&gt;/&lt;name&gt;/meta.yaml</code>（面板据此显示「已安装」并可卸载）。
            若脚本中途失败（如 <code>mysqld --initialize-insecure</code> 缺 <code>libaio.so.1</code>），磁盘上已留下安装目录 / 软链 / 服务单元，
            而面板仍显示「未安装」→ 卸载按钮不可用；此时脚本若只按「安装目录在不在」守卫并报「已安装」，
            就形成<strong>装不了也卸不掉</strong>的死锁。约定做法：
          </p>
          <ul>
            <li>以脚本末尾登记的 <code>APP_PATH/info.yaml</code> 为「装完了」的唯一依据（<code>app_install_complete</code>），命中才拒绝重装；</li>
            <li>目录还在但没登记 = 装了一半的<strong>残局</strong> → 停服务、清目录 / 软链 / 配置后<strong>继续安装</strong>，而不是报错退出；</li>
            <li>数据目录已初始化（可能含真实数据）时<strong>绝不自动删</strong>，明确报错让人工备份后处理；</li>
            <li>各步骤做成幂等：已解压就跳过解压、数据目录已初始化就跳过初始化、软链用 <code>ln -sfn</code>、配置只在确有 <code>my.cnf</code> 时才备份。</li>
          </ul>
          <table class="doc-table">
            <thead><tr><th style="width: 300px">函数</th><th>用法与说明</th></tr></thead>
            <tbody>
              <tr><td><code>app_install_complete</code></td><td><code>APP_PATH/info.yaml</code> 存在即判「装完了」（info.yaml 由脚本末尾自行登记）</td></tr>
              <tr><td><code>db_data_initialized &lt;datadir&gt;</code></td><td>数据目录是否已完成初始化（MySQL / MariaDB 通用：<code>mysql</code> 系统库或 InnoDB 系统表空间存在）；<strong>已初始化 = 可能有真实数据，绝不自动删</strong></td></tr>
              <tr><td><code>remove_path &lt;path&gt;</code></td><td>安全删除（目录 / 文件 / 软链通吃；空路径与 <code>/</code> 一律拒绝，不存在时静默返回 0）</td></tr>
              <tr><td><code>service_stop_disable &lt;unit&gt;</code></td><td>停止并禁用服务（幂等；systemctl / service / chkconfig 都试，均缺失返回 1）</td></tr>
            </tbody>
          </table>

          <p class="sec-sub"><strong>典型脚本骨架（把上面几组函数串起来）</strong></p>
          <pre class="code">{{ codes.utilsSkeleton }}</pre>

          <p class="footnote">本文档与 <code>app.yaml</code> 解析、<code>zapexec/src/verbs/appstore.rs</code> 执行实现保持同步；如有出入以代码为准。</p>
        </main>
      </div>
    </el-card>
  </div>
</template>

<script setup lang="ts">
/**
 * 目录点击：平滑滚动到对应章节，并把锚点写入地址栏（不触发整页路由跳转/回顶）。
 * 页面实际滚动容器是 .app-main，scrollIntoView 可直接作用于该容器。
 */
function scrollToSection(id: string) {
  const el = document.getElementById(id)
  if (!el) return
  el.scrollIntoView({ behavior: 'smooth', block: 'start' })
  if (history.replaceState) history.replaceState(null, '', '#' + id)
}

const codes = {
  tree: `repos/<仓库>/
└── <category>/
    └── <name>/
        ├── app.yaml          # 应用描述（含 scripts / options / actions）
        ├── install.sh        # 安装脚本（缺省文件名）
        ├── uninstall.sh      # 卸载脚本（缺省文件名）
        ├── upgrade.sh        # 升级脚本（可选）
        └── ...               # 其余资源，随快照下发
`,
  categories: `infra          基础设施       如 nginx 等基础服务 / Web 服务器（样例 infra/nginx/）
application    应用程序       独立形态的应用与运行环境（如 php）
webapps        Web 应用程序   面向 Web 的产品应用（博客 / CMS 等）
database       数据层         数据库与存储服务（如 mariadb / mysql）
library        基础库         编译期依赖库（如 openssl / libpng / libpcre2）
`,
  model: `\$ZAP_PATH/
├── data/appstore/
│   ├── repos/<repo>/<category>/<name>/   # 仓库源（只读参考）
│   ├── runs/<run_id>/                    # 一次运行完整现场：成功后整体清理，失败保留
│   │   ├── pkg/                          # 脚本快照：app.yaml + 脚本 + options.env/json（可编辑重跑）
│   │   │   ├── app.yaml
│   │   │   ├── install.sh
│   │   │   ├── options.env               # 安装/升级选项（可 source、可编辑）
│   │   │   └── options.json
│   │   ├── build/                        # 编译目录（BUILD_PATH，随 run 一并清理）
│   │   └── run.json                      # 运行参数记录
│   └── logs/run-<run_id>.log             # 实时日志（结束含 __ZAP_DONE__ <code>）
├── data/apps/<category>/<name>/          # 安装元数据：meta.yaml（系统写）+ info.yaml（脚本写）
└── \$APPS_DIR/<name>-<ver>/              # 软件本体：默认 /usr/local/apps（ZAP_APPS_DIR 可覆盖）
`,
  infoYaml: `# 安装脚本末尾：登记实例信息（值用脚本内变量展开，勿字面写死）
cat > "\${APP_PATH}/info.yaml" <<EOF
svc_name: php-fpm-\${PHP_SHORT_VERSION}   # 守护型填 systemd unit 名；库类删除此行
instance: php\${PHP_SHORT_VERSION}
install_dir: \${PHP_INSTALL_PATH}          # 软件本体在 \$APPS_DIR 下的实际安装目录
config_file: \${PHP_INSTALL_PATH}/etc/php.ini
config_files:                 # 可选：可编辑文件列表；缺省时仅 config_file
  - path: \${PHP_INSTALL_PATH}/etc/php.ini
    label: php.ini（主配置）
  - path: \${PHP_INSTALL_PATH}/etc/php-fpm.conf
    label: php-fpm.conf
  - path: \${PHP_INSTALL_PATH}/etc/php-fpm.d/www.conf
    label: www.conf（FPM 池）
pid_file: \${PHP_FPM_PID}                  # 无守护进程的库类删除此行
expose: unix:\${PHP_FPM_SOCK}
tags:
  - language
EOF
`,
  optionsYaml: `# 动作键 install / upgrade / uninstall 可分别定义（未声明时缺省回退 install，
# uninstall 不回退：未声明 options.uninstall 即卸载不弹窗）
# 动作键的值两种写法：选项数组；或 { items, intro }
# intro：整组介绍，展示在选项表单最下方（纯展示，不注入 env）；仅有 intro 也会作为说明页弹出
# 顶层直接写数组 = 作用于全部动作
options:
  install:
    intro: 以下选项决定本次编译包含的模块与附加 configure 参数。
    items:
      - name: MODULES
        label: 编译模块
        type: multiselect
        choices: [ssl, gzip, stub_status, ipv6]
        separator: ' '
        default: ssl
        required: true
        desc: 勾选需要编译进 nginx 的模块
      - name: EXTRA_CONFIG
        label: 额外 configure 参数
        type: string
        placeholder: --with-http_v2_module
        desc: 原样拼接到 ./configure 末尾
  upgrade:
    intro: 升级会保留数据并重装目标版本，以下选项对备份与安装脚本同时生效。
    items:
      - name: BACKUP
        label: 升级前备份
        type: bool
        default: true
  uninstall:
    intro: 卸载将删除安装目录并移除服务配置，建议保留数据备份。
    items:
      - name: KEEP_DATA
        label: 保留数据
        type: bool
        default: true
        desc: 关闭后不保留数据目录
`,
  readOpts: `# 方式一：直接用注入的环境变量（env 已注入，最常用）
echo "已选模块: $MODULES"
# 安装目标通常落在 $APPS_DIR 下（各包可自行定义 INSTALL_PATH 等变量拼版本目录）
./configure --prefix="$APPS_DIR/nginx-$APP_VERSION" $EXTRA_CONFIG $MODULES || exit 1

# 方式二：source options.env（与脚本同目录）
source "$PKG_PATH/options.env"

# 方式三：读 options.json（适合 python / jq）
python3 - "$PKG_PATH/options.json" <<'PY'
import json, sys
opts = json.load(open(sys.argv[1]))
print(opts.get("MODULES", ""))
PY
`,
  nginxYaml: `# infra/nginx/app.yaml（节选）
name: nginx
category: infra              # 分类 = 仓库一级目录名
version: [1.24.0]
actions:
  build: 编译安装
scripts:
  install: build_linux_amd64.sh
options:
  build:
    - name: MODULES
      label: 编译模块
      type: multiselect
      choices:
        - ssl
        - gzip
        - stub_status
        - ipv6
      default: ssl
      required: true
      desc: 勾选需要编译进 nginx 的模块，多选以空格分隔
    - name: EXTRA_CONFIG
      label: 额外 configure 参数
      type: string
      desc: 原样追加到 ./configure 末尾
`,
  nginxUse: `# build_linux_amd64.sh（节选）
# $MODULES / $EXTRA_CONFIG 已由系统注入 env（来自 options）
# prefix 落在 $APPS_DIR 下：仓库样例 INSTALL_PATH="$APPS_DIR/nginx-$APP_VERSION"
./configure \\
--prefix="$INSTALL_PATH" \\
--with-http_ssl_module \\
--with-http_gzip_static_module \\
\${EXTRA_CONFIG:-} \${MODULES:-} \\
|| exit 1

make -j"$CPU_NUM" || exit 1
make install || exit 1
`,
  familyYaml: `# database/mysql/app.yaml（节选）：合并入口 = 一个包承载多个家族
# 规则：同一包内版本字符串全局唯一；家族由 version_meta 一一声明
name: mysql
title: MySQL / MariaDB
version: [9.7.2, 8.4.11, 8.0.46, 12.3.3, 11.8.9, 10.11.19]
allow_multiple_instances: 'no'
scripts:
  install: install.sh          # 入口脚本：按 APP_FAMILY 分流到家族子脚本
version_meta:
  # 展示名（label / group）无前端内置映射，全由这里声明
  # label：短名（下拉选项内标签、卡片选中态）；group：下拉分组标题
  "9.7.2":    { family: mysql,   label: MySQL,   group: MySQL Community Server }
  "8.4.11":   { family: mysql,   label: MySQL,   group: MySQL Community Server }
  "12.3.3":   { family: mariadb, label: MariaDB, group: MariaDB Server }
  "10.11.19": { family: mariadb, label: MariaDB, group: MariaDB Server }
`,
  familyInstall: `#!/bin/bash
# install.sh —— 合并入口：只按 APP_FAMILY 分流，绝不按版本号猜家族
# APP_FAMILY 由 zapexec 依 app.yaml version_meta 注入（install/uninstall/upgrade 均注入）
set -euo pipefail

case "\${APP_FAMILY:-}" in
    mysql)   exec "\${BASH_SOURCE[0]%/*}/mysql-install.sh" ;;
    mariadb) exec "\${BASH_SOURCE[0]%/*}/mariadb-install.sh" ;;
    *)
        echo "[mysql-mariadb] 无法确定家族: APP_FAMILY=\${APP_FAMILY:-空} (\${APP_VERSION:-unknown})"
        echo "[mysql-mariadb] 请检查 app.yaml version_meta 是否声明了该版本的 family"
        exit 1
        ;;
esac
`,
  runtimeDeps: `#!/bin/bash
set -euo pipefail
source "\${ZAP_PATH}/scripts/zap/bash_utils.sh"

# 运行时依赖：包名随发行版 / 版本变化（Ubuntu 24.04+ 的 libaio1 已改名 libaio1t64），
# 且 apt 只要有一个包名找不到就整条命令失败、一个都不装 —— 必须按候选名逐个尝试
PKG_MGR="\$(pkg_manager || true)"
case "\${PKG_MGR}" in
    apt)
        apt-get update -y >/dev/null 2>&1 || log_warn "apt-get update 失败(继续尝试安装)"
        if ! have_lib libaio.so.1; then
            pkg_install_any apt libaio1t64 libaio1 \\
                || log_warn "libaio 包未装上，尝试用已有库做兼容软链"
            # libaio1t64 只提供 libaio.so.1t64，官方二进制按 libaio.so.1 加载 → 补同名软链
            link_lib_compat libaio.so.1 libaio.so.1t64 \\
                || { log_error "缺少 libaio.so.1：mysqld 必需"; exit 1; }
        fi
        # 客户端按 libncurses.so.6 加载；装不上时用已有的 libncursesw.so.6 兜底
        have_lib 'libncurses.so.*' \\
            || pkg_install_any apt libncurses6 libncurses5 \\
            || link_lib_compat libncurses.so.6 libncursesw.so.6 \\
            || { log_error "缺少 libncurses：mysql 客户端必需"; exit 1; }
        ;;
    dnf | yum)
        have_lib libaio.so.1 \\
            || pkg_install_any "\${PKG_MGR}" libaio \\
            || { log_error "缺少 libaio.so.1：mysqld 必需"; exit 1; }
        ;;
    *)
        log_warn "未识别的包管理器：请自行确认依赖已安装"
        ;;
esac
`,
  utilsSkeleton: `#!/bin/bash
set -euo pipefail
source "\${ZAP_PATH}/scripts/zap/bash_utils.sh"

# 1) 装完了就别重装(残局另见下文:目录在但没登记 = 装了一半)
if app_install_complete; then
    log_info "\${APP_NAME} 已安装,跳过"
    exit 0
fi

# 2) 前置:运行用户 + 关键目录 + 首次系统编译依赖
prepare_install_env mysql

# 3) 下载 → 解压(下不到就中止,没必要继续)
download_file "\${PKG_URL}" "\${PKG_PATH}/pkg.tar.gz"
extract_archive "\${PKG_PATH}/pkg.tar.gz" "\${BUILD_PATH}"

# 4) 编译(并行失败自动回退串行)
cd "\${BUILD_PATH}/xxx"
./configure --prefix="\${APP_PATH}"
MakeInstall

# 5) 按版本走分支(点分版本比较,忽略字母后缀)
if version_ge "\${APP_VERSION}" "8.4"; then
    log_info "8.4+ 走新参数"
fi

# 6) 初始化(幂等:已初始化就跳过;已初始化的数据目录绝不自动删)
if ! db_data_initialized "\${DATA_DIR}"; then
    "\${APP_PATH}/bin/mysqld" --initialize-insecure --user=mysql
fi

# 7) 登记:写完 info.yaml 之后 app_install_complete 才为真
printf 'version: %s\\ninstall_dir: %s\\n' "\${APP_VERSION}" "\${APP_PATH}" > "\${APP_PATH}/info.yaml"
log_ok "安装完成: \${APP_PATH}"
`,
}
</script>

<script lang="ts">
const toc = [
  { id: 'sec-package', label: '一、包结构' },
  { id: 'sec-appyaml', label: '二、app.yaml 字段' },
  { id: 'sec-lifecycle', label: '三、生命周期与升级' },
  { id: 'sec-model', label: '四、执行模型' },
  { id: 'sec-env', label: '五、环境变量' },
  { id: 'sec-options', label: '六、options 定义' },
  { id: 'sec-read', label: '七、脚本如何读取' },
  { id: 'sec-limits', label: '八、命名与长度限制' },
  { id: 'sec-example', label: '九、完整示例' },
  { id: 'sec-family', label: '十、合并入口示例' },
  { id: 'sec-trouble', label: '十一、失败排查' },
  { id: 'sec-runas', label: '十二、运行身份与脚本语言' },
  { id: 'sec-provision', label: '十三、建站 / 建库编排' },
  { id: 'sec-utils', label: '十四、bash_utils 函数库' },
]
export default { name: 'DevAppScriptGuide' }
</script>

<style scoped>
.app-script-guide .el-card {
  border: none;
}
.card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.header-left {
  display: flex;
  align-items: center;
}
.title {
  font-size: 15px;
  font-weight: 600;
}
.guide-body {
  display: flex;
  gap: 24px;
  align-items: flex-start;
}
.guide-toc {
  position: sticky;
  top: 8px;
  flex: 0 0 200px;
  border-right: 1px solid var(--el-border-color-lighter);
  padding: 4px 16px 16px 0;
}
.toc-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--el-text-color-primary);
  margin-bottom: 8px;
}
.toc-item {
  display: block;
  font-size: 13px;
  color: var(--el-text-color-regular);
  line-height: 2;
  text-decoration: none;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.toc-item:hover {
  color: var(--el-color-primary);
}
.guide-content {
  flex: 1;
  min-width: 0;
  max-width: 900px;
}
.lead {
  color: var(--el-text-color-secondary);
}
.guide-content h2 {
  font-size: 17px;
  margin: 28px 0 12px;
  padding-bottom: 8px;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.guide-content p,
.guide-content li {
  font-size: 13.5px;
  line-height: 1.9;
  color: var(--el-text-color-regular);
}
.guide-content code {
  font-family: 'JetBrains Mono', Consolas, Monaco, monospace;
  font-size: 12.5px;
  background: var(--el-fill-color-light);
  color: var(--el-color-primary);
  padding: 1px 5px;
  border-radius: 4px;
}
pre.code {
  background: var(--el-fill-color-light);
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 12px 14px;
  font-family: 'JetBrains Mono', Consolas, Monaco, monospace;
  font-size: 12.5px;
  line-height: 1.75;
  overflow-x: auto;
  color: var(--el-text-color-primary);
  white-space: pre;
}
.sec-sub {
  margin: 16px 0 10px;
  font-size: 14px;
  color: var(--el-text-color-primary);
}
.doc-tip {
  margin: 12px 0;
}
.doc-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  margin: 12px 0;
}
.doc-table th,
.doc-table td {
  border: 1px solid var(--el-border-color-lighter);
  padding: 8px 10px;
  text-align: left;
  vertical-align: top;
  line-height: 1.7;
}
.doc-table th {
  background: var(--el-fill-color-light);
  font-weight: 600;
  white-space: nowrap;
}
.footnote {
  margin-top: 32px;
  font-size: 12px;
  color: var(--el-text-color-placeholder);
}
@media (max-width: 1100px) {
  .guide-toc {
    display: none;
  }
}
</style>
