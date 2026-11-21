'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const { collectIssueLabels, applyIssueLabels, FORM_FIELDS, KEYWORD_RULES } = require('./issue_labels.cjs');
const { syncLabels } = require('./sync_labels.cjs');

const areaHeading = '涉及模块 / Affected area';
const platformHeading = '涉及平台 / Affected platforms';
const osHeading = '操作系统 / Operating system';
const bugBody = (area, platform = '不涉及在线平台 / Not related to online platforms', os = '') =>
  `### 问题描述 / Problem description\n\n无法完成操作\n\n### ${areaHeading}\n\n${area}\n\n### ${platformHeading}\n\n${platform}` +
  (os ? `\n\n### ${osHeading}\n\n${os}` : '');

test('bug 表单按明确选择补充类型、分诊、模块、平台和系统标签', () => {
  assert.deepEqual(collectIssueLabels({
    body: bugBody('播放与音频 / Playback and audio', '网易云音乐 / NetEase Cloud Music, QQ 音乐 / QQ Music', 'Windows'),
    labels: [],
  }), ['bug', 'status: needs-triage', 'area: playback', 'platform: netease', 'platform: qq', 'os: windows']);
});

test('功能与建议表单使用 enhancement', () => {
  for (const heading of ['功能背景与需求 / Motivation', '建议概述 / Suggestion overview']) {
    assert.deepEqual(collectIssueLabels({ body: `### ${heading}\n\n希望改进体验`, labels: [] }),
      ['enhancement', 'status: needs-triage']);
  }
});

test('精简功能请求不填平台时仍按多选模块分类，人工分诊后只补模块', () => {
  const body = '### 功能背景与需求 / Motivation\n\n希望改进使用体验' +
    '\n\n### 期望的功能 / Proposed feature\n\n支持自定义显示方式' +
    `\n\n### ${areaHeading}\n\n歌词 / Lyrics, 界面与窗口 / UI and window`;
  for (const platform of ['', `\n\n### ${platformHeading}\n\n_No response_`]) {
    assert.deepEqual(collectIssueLabels({ body: body + platform, labels: [] }),
      ['enhancement', 'status: needs-triage', 'area: lyrics', 'area: ui']);
    assert.deepEqual(collectIssueLabels({ body: body + platform, labels: ['enhancement', 'P1', 'status: confirmed'] }),
      ['area: lyrics', 'area: ui']);
  }
});

test('功能请求可从正文识别平台，表单平台优先', () => {
  const body = '### 功能背景与需求 / Motivation\n\n希望支持 Bilibili 视频播放' +
    `\n\n### ${areaHeading}\n\n播放与音频 / Playback and audio`;
  assert.deepEqual(collectIssueLabels({ body, labels: [] }),
    ['enhancement', 'status: needs-triage', 'area: playback', 'platform: bilibili']);
  assert.deepEqual(collectIssueLabels({ body: body + `\n\n### ${platformHeading}\n\nYouTube Music`, labels: [] }),
    ['enhancement', 'status: needs-triage', 'area: playback', 'platform: youtube']);
});

test('已有 question 标签阻止从表单标题添加 bug 或 enhancement', () => {
  for (const body of [bugBody('其他或不确定 / Other or unsure'), '### 功能背景与需求 / Motivation\n\n需要帮助']) {
    assert.deepEqual(collectIssueLabels({ body, labels: ['question', 'P1', 'status: confirmed'] }), []);
  }
});

test('未知选项、未回答和继承属性名不会生成标签', () => {
  assert.deepEqual(collectIssueLabels({ body: bugBody('P0, __proto__, constructor', '_No response_', 'toString'), labels: [] }),
    ['bug', 'status: needs-triage']);
});

test('日志和分类字段中的关键词与恶意代码不会参与推断或执行', () => {
  global.issueLabelsInjected = false;
  const body = bugBody('${global.issueLabelsInjected = true}', 'platform: youtube') +
    '\n\n### 日志 / Logs\n\nP0 crash urgent area: auth\n```js\n### 涉及模块 / Affected area\n\n账号与登录 / Accounts and login\n```';
  assert.deepEqual(collectIssueLabels({ body, labels: [] }), ['bug', 'status: needs-triage']);
  assert.equal(global.issueLabelsInjected, false);
  delete global.issueLabelsInjected;
});

test('围栏内的伪表单和重复字段不会伪造自动分类', () => {
  assert.deepEqual(collectIssueLabels({ body: '```\n' + bugBody('歌词 / Lyrics') + '\n```', labels: [] }),
    ['status: needs-triage']);
  assert.deepEqual(collectIssueLabels({ body: bugBody('歌词 / Lyrics') + `\n\n### ${areaHeading}\n\n账号与登录 / Accounts and login`, labels: [] }),
    ['bug', 'status: needs-triage']);
});

test('保留已有人工分诊分组和优先级，不新增冲突分类', () => {
  assert.deepEqual(collectIssueLabels({
    body: bugBody('歌词 / Lyrics', '网易云音乐 / NetEase Cloud Music', 'macOS'),
    labels: [{ name: 'enhancement' }, { name: 'status: confirmed' }, { name: 'area: ui' },
      { name: 'platform: youtube' }, { name: 'os: linux' }, { name: 'P1' }, { name: 'custom' }],
  }), []);
});

test('已存在的标签不重复添加，未分诊分组仍可补齐', () => {
  assert.deepEqual(collectIssueLabels({ body: bugBody('歌词 / Lyrics'), labels: ['BUG', 'custom'] }),
    ['status: needs-triage', 'area: lyrics']);
});

test('多选模块去重并给文档选项添加 documentation 标签', () => {
  assert.deepEqual(collectIssueLabels({ body: bugBody('歌词 / Lyrics, 文档与翻译 / Documentation and translation, 歌词 / Lyrics'), labels: [] }),
    ['bug', 'status: needs-triage', 'area: lyrics', 'area: documentation', 'documentation']);
});

test('所有模块选项都有显式映射，优先级不由表单自动设置', () => {
  const cases = [
    ['账号与登录 / Accounts and login', ['area: auth']],
    ['播放与音频 / Playback and audio', ['area: playback']],
    ['音频输出设备 / Audio output devices', ['area: audio-output']],
    ['系统媒体控制与快捷键 / System media controls and shortcuts', ['area: media-controls']],
    ['网络与代理 / Network and proxy', ['area: network']],
    ['歌词 / Lyrics', ['area: lyrics']],
    ['下载与存储 / Downloads and storage', ['area: download']],
    ['本地音乐与歌单 / Local music and playlists', ['area: local-media']],
    ['云同步与备份 / Cloud sync and backup', ['area: sync']],
    ['一起听 / Listen Together', ['area: listen-together']],
    ['界面与窗口 / UI and window', ['area: ui']],
    ['性能与资源占用 / Performance and resource usage', ['area: performance']],
    ['文档与翻译 / Documentation and translation', ['area: documentation', 'documentation']],
    ['构建、CI 与发布 / Build and CI and release', ['area: ci']],
    ['安装、启动与更新 / Installation and startup and update', ['area: startup']],
    ['其他或不确定 / Other or unsure', []],
  ];
  for (const [choice, labels] of cases) {
    assert.deepEqual(collectIssueLabels({ body: bugBody(choice), labels: [] }), ['bug', 'status: needs-triage', ...labels], choice);
  }
  assert.equal(cases.length, Object.keys(FORM_FIELDS.area.options).length);
});

test('所有平台选项都有显式映射', () => {
  const cases = [
    ['网易云音乐 / NetEase Cloud Music', ['platform: netease']],
    ['QQ 音乐 / QQ Music', ['platform: qq']],
    ['Bilibili', ['platform: bilibili']],
    ['YouTube Music', ['platform: youtube']],
    ['本地音频或下载文件 / Local audio or downloaded files', []],
    ['不涉及在线平台 / Not related to online platforms', []],
    ['其他或不确定 / Other or unsure', []],
  ];
  for (const [choice, labels] of cases) {
    assert.deepEqual(collectIssueLabels({ body: bugBody('其他或不确定 / Other or unsure', choice), labels: [] }), ['bug', 'status: needs-triage', ...labels], choice);
  }
  assert.equal(cases.length, Object.keys(FORM_FIELDS.platform.options).length);
});

test('所有系统选项都有显式映射，占位提示不产生标签', () => {
  const cases = [
    ['Windows', ['os: windows']],
    ['macOS', ['os: macos']],
    ['Linux', ['os: linux']],
    ['其他或不确定 / Other or unsure', []],
  ];
  for (const [choice, labels] of cases) {
    assert.deepEqual(collectIssueLabels({ body: bugBody('其他或不确定 / Other or unsure', undefined, choice), labels: [] }),
      ['bug', 'status: needs-triage', ...labels], choice);
  }
  assert.equal(cases.length, Object.keys(FORM_FIELDS.os.options).length);
  assert.deepEqual(collectIssueLabels({ body: bugBody('其他或不确定 / Other or unsure', undefined, '请选择 / Please select'), labels: [] }),
    ['bug', 'status: needs-triage']);
});

test('正文提到系统名称不会推断系统标签', () => {
  assert.deepEqual(collectIssueLabels({ title: 'Looks like Windows on Linux', body: 'macOS style', labels: [] }),
    ['status: needs-triage']);
});

// 解析表单里的下拉字段，用于校验模板与脚本协议一致
function dropdownFields(source) {
  return source.split(/\n(?= {2}- type: )/).filter(block => block.startsWith('  - type: dropdown')).map(block => ({
    title: block.match(/^ {6}label: (.+)$/m)?.[1],
    choices: [...block.matchAll(/^ {8}- (.+)$/gm)].map(match => match[1]),
  }));
}

test('三个表单的分类字段与脚本协议一致，所有自动标签存在于清单', () => {
  const required = {
    'bug-report.yml': ['area', 'platform', 'os'],
    'feature-request.yml': ['area', 'platform'],
    'suggestion-submission.yml': ['area', 'platform', 'os'],
  };
  for (const [filename, groups] of Object.entries(required)) {
    const source = fs.readFileSync(path.join(__dirname, '..', 'ISSUE_TEMPLATE', filename), 'utf8');
    const fields = dropdownFields(source);
    for (const group of groups) {
      const field = FORM_FIELDS[group];
      const matching = fields.filter(item => item.title === field.title);
      assert.equal(matching.length, 1, `${filename}: ${field.title}`);
      // 单选字段首项是中性占位提示，不参与映射
      const choices = matching[0].choices.filter(choice => choice !== '请选择 / Please select');
      assert.deepEqual(choices, Object.keys(field.options), filename);
    }
  }
  const definitions = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'labels.json'), 'utf8'));
  const names = new Set(definitions.map(label => label.name));
  for (const field of Object.values(FORM_FIELDS)) {
    for (const labels of Object.values(field.options)) {
      for (const name of labels) assert.ok(names.has(name), name);
    }
  }
  for (const name of ['bug', 'enhancement', 'question', 'documentation', 'status: needs-triage', 'P0', 'P1', 'P2', 'P3']) assert.ok(names.has(name), name);
  for (const rule of KEYWORD_RULES) assert.ok(names.has(rule.label), rule.label);
});

test('播放分类上可叠加输出设备与系统媒体控制主题', () => {
  const body = bugBody('播放与音频 / Playback and audio').replace('无法完成操作', '切换到 USB DAC 输出设备后，媒体键失效');
  assert.deepEqual(collectIssueLabels({ title: '[Bug] 切换设备后失灵', body, labels: [] }),
    ['bug', 'status: needs-triage', 'area: playback', 'area: audio-output', 'area: media-controls']);
});

test('英文叙述识别系统媒体控制与输出设备', () => {
  const body = bugBody('播放与音频 / Playback and audio').replace('无法完成操作',
    'SMTC overlay shows the wrong track and the output device resets after PipeWire restarts.');
  assert.deepEqual(collectIssueLabels({ body, labels: [] }),
    ['bug', 'status: needs-triage', 'area: playback', 'area: audio-output', 'area: media-controls']);
});

test('无表单时标题与正文可识别明确模块和平台', () => {
  assert.deepEqual(collectIssueLabels({ title: 'Bilibili download tasks stay pending', body: 'Downloads never finish.', labels: [] }),
    ['status: needs-triage', 'area: download', 'platform: bilibili']);
  assert.deepEqual(collectIssueLabels({ title: 'WebDAV backup fails with QQ Music', body: '', labels: [] }),
    ['status: needs-triage', 'area: sync', 'platform: qq']);
});

test('表单选择其他或不确定时仍从叙述补充模块和平台', () => {
  const body = bugBody('其他或不确定 / Other or unsure', '其他或不确定 / Other or unsure')
    .replace('无法完成操作', 'Bilibili 下载任务始终没有完成');
  assert.deepEqual(collectIssueLabels({ body, labels: [] }),
    ['bug', 'status: needs-triage', 'area: download', 'platform: bilibili']);
});

test('明确叙述词可回退分类', () => {
  for (const [title, label] of [
    ['登录二维码失效', 'area: auth'], ['播放音效失效', 'area: playback'],
    ['声卡没有输出', 'area: audio-output'], ['全局快捷键无效', 'area: media-controls'],
    ['代理无法连接', 'area: network'], ['逐字歌词时间错误', 'area: lyrics'],
    ['本地音乐歌单缺失', 'area: local-media'], ['Listen Together room disconnected', 'area: listen-together'],
    ['标题栏按钮错位', 'area: ui'], ['内存占用过高', 'area: performance'],
    ['README 文档缺少说明', 'area: documentation'], ['cargo 编译失败', 'area: ci'],
    ['安装后白屏', 'area: startup'], ['YouTube Music request fails', 'platform: youtube'],
    ['网易云每日推荐为空', 'platform: netease'],
  ]) assert.ok(collectIssueLabels({ title, body: '', labels: [] }).includes(label), `${title}: ${label}`);
});

test('模板固定字段、安装来源和确认项不会推断功能标签', () => {
  const body = bugBody('其他或不确定 / Other or unsure') +
    '\n\n### 复现时的音频输出设备 / Audio output during reproduction\n\nUSB DAC 或声卡 / USB DAC or audio interface' +
    '\n\n### 安装包类型 / Package format\n\nAppImage' +
    '\n\n### 安装来源 / Distribution\n\nGitHub Actions CI' +
    '\n\n### 提交前确认 / Confirmation\n\n- [x] 我已在最新 Release 或 CI 构建中复现问题';
  assert.deepEqual(collectIssueLabels({ title: '[Bug] ', body, labels: [] }), ['bug', 'status: needs-triage']);
});

test('围栏、注释、引用、内联代码和日志行不触发关键词', () => {
  const body = bugBody('其他或不确定 / Other or unsure').replace('无法完成操作', [
    '```text', 'SMTC WebDAV PipeWire', '```',
    '<!-- YouTube lyrics -->', '> 媒体键、输出设备、下载', '',
    '`USB DAC`', '- [x] CI 构建验证过了',
    '2026-10-25 12:00:00 ERROR output device lost',
    '[2026-10-25][12:00:00][neri_playback][ERROR] download failed',
    "thread 'main' panicked at crates/neri-playback/src/remote/mod.rs",
  ].join('\n')) + '\n\n### 日志 / Logs\n\nSMTC MPRIS WebDAV';
  assert.deepEqual(collectIssueLabels({ body, labels: [] }), ['bug', 'status: needs-triage']);
});

test('英文单词边界不把普通单词中的片段误当标签', () => {
  assert.deepEqual(collectIssueLabels({ title: 'Special principle and coincidental precision', body: 'social facial efficiency', labels: [] }),
    ['status: needs-triage']);
  assert.deepEqual(collectIssueLabels({ title: 'CI workflow fails', body: '', labels: [] }),
    ['status: needs-triage', 'area: ci']);
});

test('已有人工分诊保留，仅补可叠加主题，不改 P1 与 confirmed', () => {
  const issue = { title: '切换输出设备后歌词异常', body: 'YouTube Music download also fails.',
    labels: ['bug', 'P1', 'status: confirmed', 'area: playback', 'platform: netease', 'custom'] };
  assert.deepEqual(collectIssueLabels(issue), ['area: audio-output']);
});

test('普通叙述不推断优先级或确认状态', () => {
  assert.deepEqual(collectIssueLabels({ title: 'P0 urgent confirmed', body: 'Please mark P1 status: confirmed.', labels: [] }),
    ['status: needs-triage']);
});

test('Issue runner 读取最新标签，只补充缺少项且不在日志中输出正文', async () => {
  const requests = [];
  const logs = [];
  const result = await applyIssueLabels({
    context: { repo: { owner: 'owner', repo: 'repo' }, payload: { action: 'opened', issue: { number: 12 } } },
    github: { rest: { issues: {
      get: async args => {
        assert.deepEqual(args, { owner: 'owner', repo: 'repo', issue_number: 12 });
        return { data: { number: 12, body: bugBody('歌词 / Lyrics').replace('无法完成操作', 'private-description-marker'), labels: [{ name: 'P1' }, { name: 'status: confirmed' }] } };
      },
      addLabels: async args => { requests.push(args); },
    } } },
    core: { info(message) { logs.push(message); } },
  });
  assert.deepEqual(result, ['bug', 'area: lyrics']);
  assert.deepEqual(requests, [{ owner: 'owner', repo: 'repo', issue_number: 12, labels: ['bug', 'area: lyrics'] }]);
  assert.ok(logs[0].includes('area: lyrics [form:area]'));
  assert.ok(!logs[0].includes('private-description-marker'));
});

test('编辑、重开和 PR 不触发自动标签 API', async () => {
  for (const payload of [{ action: 'edited', issue: { number: 12 } }, { action: 'reopened', issue: { number: 12 } },
    { action: 'opened', issue: { number: 12, pull_request: {} } }]) {
    assert.deepEqual(await applyIssueLabels({ context: { payload }, github: {}, core: {} }), []);
  }
});

test('标签已齐全时不发送空写请求', async () => {
  assert.deepEqual(await applyIssueLabels({
    context: { repo: { owner: 'o', repo: 'r' }, payload: { action: 'opened', issue: { number: 1 } } },
    github: { rest: { issues: { get: async () => ({ data: { body: '', labels: ['status: confirmed'] } }) } } },
    core: { info() {} },
  }), []);
});

test('同步标签只创建或更新清单中的项，未知远端标签被保留', async () => {
  const writes = [];
  const api = {
    listLabelsForRepo() {},
    createLabel: async args => { writes.push(['create', args]); },
    updateLabel: async args => { writes.push(['update', args]); },
  };
  const result = await syncLabels({
    context: { repo: { owner: 'o', repo: 'r' } },
    github: { rest: { issues: api }, paginate: async (endpoint, args) => {
      assert.equal(endpoint, api.listLabelsForRepo);
      assert.deepEqual(args, { owner: 'o', repo: 'r', per_page: 100 });
      return [{ name: 'bug', color: 'd73a4a', description: 'existing' }, { name: 'P1', color: 'ffffff', description: '' }, { name: 'custom', color: '123456', description: 'keep' }];
    } },
    core: { info() {} },
  }, [{ name: 'bug', color: 'd73a4a', description: 'existing' }, { name: 'P1', color: 'b60205', description: 'high' }, { name: 'P2', color: 'fbca04', description: 'normal' }]);
  assert.deepEqual(result, { created: ['P2'], updated: ['P1'], unchanged: ['bug'] });
  assert.deepEqual(writes, [
    ['update', { owner: 'o', repo: 'r', name: 'P1', new_name: 'P1', color: 'b60205', description: 'high' }],
    ['create', { owner: 'o', repo: 'r', name: 'P2', color: 'fbca04', description: 'normal' }],
  ]);
});

test('无效或重复标签配置在调用 GitHub 前拒绝', async () => {
  for (const labels of [[{ name: 'bad', color: 'red' }], [{ name: 'P1', color: '123456' }, { name: 'p1', color: '654321' }]]) {
    await assert.rejects(syncLabels({ github: {}, context: {}, core: {} }, labels), /label/i);
  }
});

test('标签配置的额外属性不能改变同步目标仓库', async () => {
  const writes = [];
  await syncLabels({
    context: { repo: { owner: 'o', repo: 'r' } },
    github: { paginate: async () => [], rest: { issues: {
      listLabelsForRepo() {}, createLabel: async args => { writes.push(args); },
    } } },
    core: { info() {} },
  }, [{ name: 'P1', color: 'b60205', description: 'high', owner: 'other', repo: 'other' }]);
  assert.deepEqual(writes, [{ owner: 'o', repo: 'r', name: 'P1', color: 'b60205', description: 'high' }]);
});

test('仓库标签清单可直接通过同步校验', async () => {
  const result = await syncLabels({
    context: { repo: { owner: 'o', repo: 'r' } },
    github: { paginate: async () => [], rest: { issues: { listLabelsForRepo() {}, createLabel: async () => {} } } },
    core: { info() {} },
  });
  const definitions = JSON.parse(fs.readFileSync(path.join(__dirname, '..', 'labels.json'), 'utf8'));
  assert.equal(result.created.length, definitions.length);
});
