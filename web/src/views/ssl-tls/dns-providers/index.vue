<!--
  DNS 服务商凭据：独立页面。

  侧栏入口已收起（`menus.hidden = 1`）——SSL/TLS 现在是一级单菜单，
  这个页面改由证书列表页的「DNS 服务商」抽屉承载。
  路由保留是为了让旧链接 /ssl-tls/dns-providers 仍然可用。
-->
<template>
  <DnsProvidersPane />
</template>

<script setup lang="ts">
import DnsProvidersPane from './DnsProvidersPane.vue'
</script>
