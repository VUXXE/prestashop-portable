<?php
/**
 * PrestaShop Portable Build-time Core Patcher
 * Patches PrestaShop core files to fix Windows installation and module caching issues.
 */

$appDir = $argv[1] ?? null;
if (!$appDir || !is_dir($appDir)) {
    echo "Usage: php ci/patch-prestashop.php <path-to-app-dir>\n";
    exit(1);
}

$appDir = rtrim($appDir, '/\\');
echo "Patching PrestaShop core in: $appDir\n";

$patches = [
    // 1. install/init.php
    [
        'file' => $appDir . '/install/init.php',
        'find' => "require_once 'install_version.php';",
        'replace' => "require_once 'install_version.php';\n\nif (!defined('PS_INSTALLATION_IN_PROGRESS')) {\n    define('PS_INSTALLATION_IN_PROGRESS', true);\n}",
        'check' => "if (!defined('PS_INSTALLATION_IN_PROGRESS')) {",
    ],
    [
        'file' => $appDir . '/install/init.php',
        'find' => "define('PS_INSTALLATION_IN_PROGRESS', true);",
        'replace' => "if (!defined('PS_INSTALLATION_IN_PROGRESS')) {\n    define('PS_INSTALLATION_IN_PROGRESS', true);\n}",
        'check' => "if (!defined('PS_INSTALLATION_IN_PROGRESS')) {\n    define('PS_INSTALLATION_IN_PROGRESS', true);\n}",
    ],
    // 2. app/AppKernel.php
    [
        'file' => $appDir . '/app/AppKernel.php',
        'find' => "\$this->getEnvironment() === 'test'",
        'replace' => "\$this->getEnvironment() === 'test' || defined('PS_INSTALLATION_IN_PROGRESS')",
        'check' => "defined('PS_INSTALLATION_IN_PROGRESS')",
    ],
    // 3. src/Adapter/Container/ContainerParametersExtension.php
    [
        'file' => $appDir . '/src/Adapter/Container/ContainerParametersExtension.php',
        'find' => "\$this->environment->getName() === 'test'",
        'replace' => "\$this->environment->getName() === 'test' || defined('PS_INSTALLATION_IN_PROGRESS')",
        'check' => "defined('PS_INSTALLATION_IN_PROGRESS')",
    ],
    // 4. src/Adapter/ContainerBuilder.php
    [
        'file' => $appDir . '/src/Adapter/ContainerBuilder.php',
        'find' => "\$this->environment->getName() === 'test'",
        'replace' => "\$this->environment->getName() === 'test' || defined('PS_INSTALLATION_IN_PROGRESS')",
        'check' => "defined('PS_INSTALLATION_IN_PROGRESS')",
    ],
    // 5. src/Adapter/Module/Repository/CachedModuleRepository.php
    [
        'file' => $appDir . '/src/Adapter/Module/Repository/CachedModuleRepository.php',
        'find' => "    public function getInstalledModules(): array\n    {\n        return \$this->cache->get('installed_modules', function () {\n            return \$this->decorated->getInstalledModules();\n        });\n    }\n\n    public function getPresentModules(): array\n    {\n        return \$this->cache->get('present_modules', function () {\n            return \$this->decorated->getPresentModules();\n        });\n    }\n\n    public function getActiveModules(): array\n    {\n        return \$this->cache->get('active_modules', function () {\n            return \$this->decorated->getActiveModules();\n        });\n    }",
        'replace' => "    public function getInstalledModules(): array\n    {\n        if (defined('PS_INSTALLATION_IN_PROGRESS')) {\n            return \$this->decorated->getInstalledModules();\n        }\n\n        \$installed = \$this->cache->get('installed_modules', function () {\n            return \$this->decorated->getInstalledModules();\n        });\n\n        if (empty(\$installed)) {\n            \$fresh = \$this->decorated->getInstalledModules();\n            if (!empty(\$fresh)) {\n                \$this->cache->delete('installed_modules');\n\n                return \$this->cache->get('installed_modules', function () use (\$fresh) {\n                    return \$fresh;\n                });\n            }\n        }\n\n        return \$installed;\n    }\n\n    public function getPresentModules(): array\n    {\n        if (defined('PS_INSTALLATION_IN_PROGRESS')) {\n            return \$this->decorated->getPresentModules();\n        }\n\n        return \$this->cache->get('present_modules', function () {\n            return \$this->decorated->getPresentModules();\n        });\n    }\n\n    public function getActiveModules(): array\n    {\n        if (defined('PS_INSTALLATION_IN_PROGRESS')) {\n            return \$this->decorated->getActiveModules();\n        }\n\n        \$active = \$this->cache->get('active_modules', function () {\n            return \$this->decorated->getActiveModules();\n        });\n\n        if (empty(\$active)) {\n            \$fresh = \$this->decorated->getActiveModules();\n            if (!empty(\$fresh)) {\n                \$this->cache->delete('active_modules');\n\n                return \$this->cache->get('active_modules', function () use (\$fresh) {\n                    return \$fresh;\n                });\n            }\n        }\n\n        return \$active;\n    }\n\n    public function clearCache(): bool\n    {\n        if (\$this->cache instanceof \\Symfony\\Component\\Cache\\Adapter\\AdapterInterface) {\n            return \$this->cache->clear();\n        }\n\n        return true;\n    }",
        'check' => "public function clearCache(): bool",
    ],
    // 6. install/controllers/http/process.php
    [
        'file' => $appDir . '/install/controllers/http/process.php',
        'find' => "} catch (\\Exception \$e) {",
        'replace' => "} catch (\\Throwable \$e) {",
        'check' => "catch (\\Throwable \$e)",
    ],
    [
        'file' => $appDir . '/install/controllers/http/process.php',
        'find' => "\$this->session->process_validated = array_merge(\$this->session->process_validated, ['installModules' => true]);\n        \$this->ajaxJsonAnswer(true);",
        'replace' => "\$this->session->process_validated = array_merge(\$this->session->process_validated, ['installModules' => true]);\n\n        try {\n            \$adminModulesCache = _PS_ROOT_DIR_ . '/var/cache/' . _PS_ENV_ . '/admin/modules';\n            if (is_dir(\$adminModulesCache)) {\n                (new \\Symfony\\Component\\Filesystem\\Filesystem())->remove(\$adminModulesCache);\n            }\n        } catch (\\Throwable) {\n        }\n\n        \$this->ajaxJsonAnswer(true);",
        'check' => "\$adminModulesCache = _PS_ROOT_DIR_",
    ],
    // 7. src/PrestaShopBundle/Install/Install.php
    [
        'file' => $appDir . '/src/PrestaShopBundle/Install/Install.php',
        'find' => "        \$key = PhpEncryption::createNewRandomKey();\n        \$privateKey = openssl_pkey_new([\n            'private_key_bits' => 2048,\n            'private_key_type' => OPENSSL_KEYTYPE_RSA,\n        ]);\n        openssl_pkey_export(\$privateKey, \$apiPrivateKey);\n        \$apiPublicKey = openssl_pkey_get_details(\$privateKey)['key'];",
        'replace' => "        \$key = PhpEncryption::createNewRandomKey();\n        \$openSslConfig = [\n            'private_key_bits' => 2048,\n            'private_key_type' => OPENSSL_KEYTYPE_RSA,\n        ];\n        \$openSslCnf = _PS_ROOT_DIR_ . '/config/openssl.cnf';\n        if (!file_exists(\$openSslCnf)) {\n            \$openSslCnf = dirname(_PS_ROOT_DIR_) . '/config/openssl.cnf';\n        }\n        if (file_exists(\$openSslCnf)) {\n            \$openSslConfig['config'] = \$openSslCnf;\n        }\n\n        \$privateKey = openssl_pkey_new(\$openSslConfig);\n        if (\$privateKey !== false) {\n            openssl_pkey_export(\$privateKey, \$apiPrivateKey, null, !empty(\$openSslConfig['config']) ? ['config' => \$openSslConfig['config']] : null);\n            \$apiPublicKey = (\$details = openssl_pkey_get_details(\$privateKey)) ? \$details['key'] : '';\n        } else {\n            \$apiPrivateKey = '';\n            \$apiPublicKey = '';\n        }",
        'check' => "\$openSslConfig = [",
    ],
];

$patchedCount = 0;
foreach ($patches as $p) {
    $file = $p['file'];
    if (!file_exists($file)) {
        continue;
    }
    $content = file_get_contents($file);
    if (strpos($content, $p['check']) !== false) {
        continue; // Already patched
    }
    // Normalize CRLF to LF for reliable matching
    $normalizedContent = str_replace("\r\n", "\n", $content);
    if (strpos($normalizedContent, $p['find']) !== false) {
        $normalizedContent = str_replace($p['find'], $p['replace'], $normalizedContent);
        file_put_contents($file, $normalizedContent);
        $patchedCount++;
        echo "Patched: " . basename($file) . "\n";
    }
}

echo "Successfully applied $patchedCount patch(es).\n";
